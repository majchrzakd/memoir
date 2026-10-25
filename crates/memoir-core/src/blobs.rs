//! Attachments on this device: one encrypted file per attachment,
//! `<dir>/<blob id>.mmr`, in the blob stream format (`crypto::blob`).
//!
//! The blob id is a keyed HMAC of the content, so the same file attached
//! twice is stored once and file names reveal nothing about the content. The
//! id is also bound into the encryption, so a file renamed on disk (or in the
//! cloud) fails to decrypt.

use std::fs::{self, File};
use std::io::{self, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};

use crate::crypto::{CryptoError, Keyring, blob};

/// Largest attachment accepted.
pub const MAX_BLOB_SIZE: u64 = 100 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum BlobError {
    #[error("the file is too large ({size} bytes; the limit is {limit} bytes)")]
    TooLarge { size: u64, limit: u64 },
    #[error("{0:?} is not a valid attachment id")]
    InvalidId(String),
    #[error("attachment {0} is missing on this device")]
    NotFound(String),
    #[error(transparent)]
    Crypto(#[from] CryptoError),
    #[error(transparent)]
    Io(#[from] io::Error),
}

pub type Result<T> = std::result::Result<T, BlobError>;

pub struct BlobStore {
    dir: PathBuf,
    keys: Keyring,
    limit: u64,
}

impl BlobStore {
    pub fn open(dir: impl Into<PathBuf>, keys: Keyring) -> io::Result<Self> {
        Self::with_limit(dir, keys, MAX_BLOB_SIZE)
    }

    pub fn with_limit(dir: impl Into<PathBuf>, keys: Keyring, limit: u64) -> io::Result<Self> {
        let dir = dir.into();
        fs::create_dir_all(&dir)?;
        Ok(Self { dir, keys, limit })
    }

    /// Stores `data` (if not already stored) and returns its id.
    pub fn put(&self, data: &[u8]) -> Result<String> {
        let size = data.len() as u64;
        if size > self.limit {
            return Err(BlobError::TooLarge {
                size,
                limit: self.limit,
            });
        }
        let id = blob::blob_id(&self.keys, data)?;
        let path = self.path(&id)?;
        if path.exists() {
            return Ok(id);
        }

        // Write to a temporary file and rename, so a crash never leaves a
        // truncated attachment under its final name.
        let tmp = path.with_extension("tmp");
        {
            let mut out = BufWriter::new(File::create(&tmp)?);
            blob::encrypt(&self.keys, &context(&id), data, &mut out)?;
            out.flush()?;
            out.get_ref().sync_all()?;
        }
        fs::rename(&tmp, &path)?;
        Ok(id)
    }

    /// Decrypts an attachment into memory.
    pub fn read(&self, id: &str) -> Result<Vec<u8>> {
        let path = self.path(id)?;
        let file = match File::open(&path) {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                return Err(BlobError::NotFound(id.to_owned()));
            }
            Err(e) => return Err(e.into()),
        };
        let mut data = Vec::new();
        blob::decrypt(&self.keys, &context(id), BufReader::new(file), &mut data)?;
        Ok(data)
    }

    pub fn contains(&self, id: &str) -> bool {
        self.path(id).is_ok_and(|p| p.exists())
    }

    /// Path of an attachment's file. Only well-formed ids are accepted, so an
    /// id taken from a URL can never point outside the directory.
    fn path(&self, id: &str) -> Result<PathBuf> {
        if !is_valid_id(id) {
            return Err(BlobError::InvalidId(id.to_owned()));
        }
        Ok(self.dir.join(format!("{id}.mmr")))
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }
}

/// Blob ids are 64 lowercase hex digits (HMAC-SHA256).
pub fn is_valid_id(id: &str) -> bool {
    id.len() == 64
        && id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Binds an encrypted file to its id (see `docs/FORMAT.md`).
fn context(id: &str) -> Vec<u8> {
    format!("blob:{id}").into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::SecretKey;

    fn store(dir: &Path) -> BlobStore {
        let keys = Keyring::derive(&SecretKey::from_bytes([3; 32]), 1);
        BlobStore::with_limit(dir.join("blobs"), keys, 1024 * 1024).unwrap()
    }

    #[test]
    fn put_read_roundtrip_and_dedup() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(dir.path());
        let data: Vec<u8> = (0..300_000).map(|i| (i % 251) as u8).collect();

        let id = s.put(&data).unwrap();
        assert!(is_valid_id(&id));
        assert!(s.contains(&id));
        assert_eq!(s.read(&id).unwrap(), data);

        assert_eq!(s.put(&data).unwrap(), id);
        assert_eq!(fs::read_dir(s.dir()).unwrap().count(), 1, "stored once");
        assert_ne!(s.put(b"other").unwrap(), id);
    }

    #[test]
    fn files_on_disk_are_ciphertext() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(dir.path());
        let secret = b"PNG pretend image with a very recognisable secret phrase";
        let id = s.put(secret).unwrap();
        let bytes = fs::read(s.dir().join(format!("{id}.mmr"))).unwrap();
        assert!(bytes.starts_with(crate::crypto::MAGIC));
        assert!(
            !bytes
                .windows(10)
                .any(|w| secret.windows(10).any(|s| s == w))
        );
    }

    #[test]
    fn renamed_or_tampered_files_fail() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(dir.path());
        let a = s.put(b"first file").unwrap();
        let b = s.put(b"second file").unwrap();

        // Swap the two files on disk.
        let (pa, pb) = (
            s.dir().join(format!("{a}.mmr")),
            s.dir().join(format!("{b}.mmr")),
        );
        let (da, db) = (fs::read(&pa).unwrap(), fs::read(&pb).unwrap());
        fs::write(&pa, &db).unwrap();
        fs::write(&pb, &da).unwrap();
        assert!(matches!(s.read(&a), Err(BlobError::Crypto(_))));
    }

    #[test]
    fn rejects_bad_ids_and_large_files() {
        let dir = tempfile::tempdir().unwrap();
        let s = store(dir.path());
        for bad in [
            "",
            "../vault",
            "..\\..\\x",
            &"g".repeat(64),
            &"A".repeat(64),
            &"a".repeat(63),
        ] {
            assert!(matches!(s.read(bad), Err(BlobError::InvalidId(_))), "{bad}");
        }
        assert!(matches!(
            s.read(&"a".repeat(64)),
            Err(BlobError::NotFound(_))
        ));
        assert!(matches!(
            s.put(&vec![0u8; 1024 * 1024 + 1]),
            Err(BlobError::TooLarge { .. })
        ));
    }
}
