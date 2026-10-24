//! Cryptography: key types, the envelope format for synced objects, and
//! streaming encryption for blobs. The byte-level format is specified in
//! `docs/FORMAT.md`; any change there must bump the format version.

pub mod blob;
pub mod envelope;
pub mod keys;
pub mod recovery;

pub use keys::{Keyring, SecretKey};
pub use recovery::RecoveryKey;

/// First bytes of every encrypted file Memoir writes.
pub const MAGIC: &[u8; 4] = b"MMR1";
/// Envelope/stream format version.
pub const FORMAT_VERSION: u8 = 1;

/// What an encrypted file contains; part of the authenticated header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Kind {
    Object = 1,
    Blob = 2,
}

#[derive(Debug, thiserror::Error)]
pub enum CryptoError {
    #[error("decryption failed: wrong key, or the data was modified")]
    Decrypt,
    #[error("not a Memoir encrypted file")]
    BadMagic,
    #[error("unsupported format version {0}")]
    UnsupportedVersion(u8),
    #[error("expected {expected:?} data, found kind {found}")]
    WrongKind { expected: Kind, found: u8 },
    #[error("encrypted with key epoch {0}, which this vault does not have")]
    UnknownEpoch(u32),
    #[error("encrypted data is truncated")]
    Truncated,
    #[error("encrypted data is malformed: {0}")]
    Malformed(&'static str),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, CryptoError>;

/// Fills a fixed-size array from the OS CSPRNG.
pub fn random_bytes<const N: usize>() -> [u8; N] {
    let mut buf = [0u8; N];
    getrandom::fill(&mut buf).expect("OS random number generator unavailable");
    buf
}

/// Parses and checks the common header prefix (magic, version, kind, epoch).
fn check_header(bytes: &[u8], expected: Kind, keys: &Keyring) -> Result<()> {
    if bytes.len() < 10 {
        return Err(CryptoError::Truncated);
    }
    if &bytes[..4] != MAGIC {
        return Err(CryptoError::BadMagic);
    }
    if bytes[4] != FORMAT_VERSION {
        return Err(CryptoError::UnsupportedVersion(bytes[4]));
    }
    if bytes[5] != expected as u8 {
        return Err(CryptoError::WrongKind {
            expected,
            found: bytes[5],
        });
    }
    let epoch = u32::from_le_bytes(bytes[6..10].try_into().expect("4 bytes"));
    if epoch != keys.epoch {
        return Err(CryptoError::UnknownEpoch(epoch));
    }
    Ok(())
}
