//! Attachments: streaming encryption and content-derived names.
//!
//! Encryption uses the STREAM construction (Hoang, Reyhanitabar, Rogaway,
//! Vizár 2015) over XChaCha20-Poly1305, so memory stays flat for large files
//! and every chunk is authenticated on its own:
//!
//! ```text
//! header  = "MMR1" | version u8 | kind u8 (=2) | epoch u32 LE | nonce prefix [19] | chunk size u32 LE
//! chunk i = XChaCha20-Poly1305(blob key, prefix || i u32 BE || last u8, plaintext chunk, aad = header || context)
//! ```
//!
//! Every chunk except the last carries exactly `chunk size` plaintext bytes;
//! the last carries 0..=chunk size and is flagged, so truncation, reordering
//! and appended data are all detected.

use std::io::{self, Read, Write};

use chacha20poly1305::XNonce;
use chacha20poly1305::aead::{Aead, Payload};
use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;
use zeroize::Zeroizing;

use super::keys::{NONCE_LEN, TAG_LEN};
use super::{
    CryptoError, FORMAT_VERSION, Keyring, Kind, MAGIC, Result, check_header, random_bytes,
};

pub const DEFAULT_CHUNK_SIZE: u32 = 64 * 1024;
const PREFIX_LEN: usize = NONCE_LEN - 5;
pub const HEADER_LEN: usize = 4 + 1 + 1 + 4 + PREFIX_LEN + 4;
const MIN_CHUNK_SIZE: u32 = 1024;
const MAX_CHUNK_SIZE: u32 = 16 * 1024 * 1024;

/// Encrypts everything `reader` yields into `writer`.
pub fn encrypt(
    keys: &Keyring,
    context: &[u8],
    reader: impl Read,
    writer: impl Write,
) -> Result<()> {
    encrypt_with(
        keys,
        context,
        DEFAULT_CHUNK_SIZE,
        random_bytes(),
        reader,
        writer,
    )
}

fn encrypt_with(
    keys: &Keyring,
    context: &[u8],
    chunk_size: u32,
    prefix: [u8; PREFIX_LEN],
    mut reader: impl Read,
    mut writer: impl Write,
) -> Result<()> {
    let mut header = Vec::with_capacity(HEADER_LEN);
    header.extend_from_slice(MAGIC);
    header.push(FORMAT_VERSION);
    header.push(Kind::Blob as u8);
    header.extend_from_slice(&keys.epoch.to_le_bytes());
    header.extend_from_slice(&prefix);
    header.extend_from_slice(&chunk_size.to_le_bytes());
    writer.write_all(&header)?;

    let aad = [&header[..], context].concat();
    let cipher = keys.blob.cipher();
    let size = chunk_size as usize;
    let mut current = Zeroizing::new(vec![0u8; size]);
    let mut next = Zeroizing::new(vec![0u8; size]);
    let mut len = read_full(&mut reader, &mut current)?;

    for index in 0u32.. {
        // Only a full chunk can have a successor; look ahead to find out.
        let next_len = if len == size {
            read_full(&mut reader, &mut next)?
        } else {
            0
        };
        let last = next_len == 0;
        let ct = cipher
            .encrypt(
                &chunk_nonce(&prefix, index, last),
                Payload {
                    msg: &current[..len],
                    aad: &aad,
                },
            )
            .expect("XChaCha20-Poly1305 encryption cannot fail for in-memory data");
        writer.write_all(&ct)?;
        if last {
            writer.flush()?;
            return Ok(());
        }
        std::mem::swap(&mut current, &mut next);
        len = next_len;
    }
    Err(CryptoError::Malformed("attachment too large"))
}

/// Decrypts a blob produced by [`encrypt`].
///
/// Plaintext is written as chunks are verified, so on error `writer` may hold
/// a prefix of the data: write to a temporary file and discard it on error.
pub fn decrypt(
    keys: &Keyring,
    context: &[u8],
    mut reader: impl Read,
    mut writer: impl Write,
) -> Result<()> {
    let mut header = [0u8; HEADER_LEN];
    if read_full(&mut reader, &mut header)? < HEADER_LEN {
        return Err(CryptoError::Truncated);
    }
    check_header(&header, Kind::Blob, keys)?;
    let prefix: [u8; PREFIX_LEN] = header[10..10 + PREFIX_LEN].try_into().expect("prefix");
    let chunk_size = u32::from_le_bytes(header[HEADER_LEN - 4..].try_into().expect("4 bytes"));
    if !(MIN_CHUNK_SIZE..=MAX_CHUNK_SIZE).contains(&chunk_size) {
        return Err(CryptoError::Malformed("chunk size out of range"));
    }

    let aad = [&header[..], context].concat();
    let cipher = keys.blob.cipher();
    let size = chunk_size as usize + TAG_LEN;
    let mut current = vec![0u8; size];
    let mut next = vec![0u8; size];
    let mut len = read_full(&mut reader, &mut current)?;
    if len < TAG_LEN {
        return Err(CryptoError::Truncated);
    }

    for index in 0u32.. {
        let next_len = if len == size {
            read_full(&mut reader, &mut next)?
        } else {
            0
        };
        let last = next_len == 0;
        let plain = Zeroizing::new(
            cipher
                .decrypt(
                    &chunk_nonce(&prefix, index, last),
                    Payload {
                        msg: &current[..len],
                        aad: &aad,
                    },
                )
                .map_err(|_| CryptoError::Decrypt)?,
        );
        writer.write_all(&plain)?;
        if last {
            writer.flush()?;
            return Ok(());
        }
        std::mem::swap(&mut current, &mut next);
        len = next_len;
    }
    Err(CryptoError::Malformed("attachment too large"))
}

/// Name for an attachment: keyed HMAC-SHA256 of its plaintext, hex-encoded.
/// Identical files dedupe, but nobody without the key can test whether a
/// given file is in the vault.
pub fn blob_id(keys: &Keyring, mut reader: impl Read) -> Result<String> {
    let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(keys.blob_id.as_bytes())
        .expect("HMAC accepts 32-byte keys");
    let mut buf = Zeroizing::new(vec![0u8; DEFAULT_CHUNK_SIZE as usize]);
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        mac.update(&buf[..n]);
    }
    Ok(mac
        .finalize()
        .into_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

fn chunk_nonce(prefix: &[u8; PREFIX_LEN], index: u32, last: bool) -> XNonce {
    let mut nonce = [0u8; NONCE_LEN];
    nonce[..PREFIX_LEN].copy_from_slice(prefix);
    nonce[PREFIX_LEN..PREFIX_LEN + 4].copy_from_slice(&index.to_be_bytes());
    nonce[NONCE_LEN - 1] = last as u8;
    XNonce::from(nonce)
}

/// Reads until `buf` is full or the reader is exhausted; returns bytes read.
fn read_full(reader: &mut impl Read, buf: &mut [u8]) -> io::Result<usize> {
    let mut filled = 0;
    while filled < buf.len() {
        match reader.read(&mut buf[filled..]) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(filled)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::SecretKey;

    const CS: u32 = MIN_CHUNK_SIZE;

    fn keys() -> Keyring {
        Keyring::derive(&SecretKey::from_bytes([3; 32]), 1)
    }

    fn data(len: usize) -> Vec<u8> {
        (0..len).map(|i| (i * 31 % 253) as u8).collect()
    }

    fn enc(plain: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        encrypt_with(&keys(), b"blob", CS, random_bytes(), plain, &mut out).unwrap();
        out
    }

    fn dec(sealed: &[u8]) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        decrypt(&keys(), b"blob", sealed, &mut out).map(|()| out)
    }

    #[test]
    fn roundtrip_around_chunk_boundaries() {
        let cs = CS as usize;
        for len in [0, 1, cs - 1, cs, cs + 1, 2 * cs, 3 * cs + 5] {
            let plain = data(len);
            let sealed = enc(&plain);
            let chunks = len.div_ceil(cs).max(1);
            assert_eq!(
                sealed.len(),
                HEADER_LEN + len + chunks * TAG_LEN,
                "len {len}"
            );
            assert_eq!(dec(&sealed).unwrap(), plain, "len {len}");
        }
    }

    #[test]
    fn default_chunk_size_roundtrip() {
        let plain = data(200_000);
        let mut sealed = Vec::new();
        encrypt(&keys(), b"blob", &plain[..], &mut sealed).unwrap();
        assert_eq!(dec(&sealed).unwrap(), plain);
    }

    #[test]
    fn detects_any_bit_flip() {
        let sealed = enc(&data(CS as usize + 10));
        for i in 0..sealed.len() {
            let mut t = sealed.clone();
            t[i] ^= 0x80;
            assert!(dec(&t).is_err(), "flip at byte {i}");
        }
    }

    #[test]
    fn detects_truncation_at_every_length() {
        let sealed = enc(&data(2 * CS as usize));
        for cut in 0..sealed.len() {
            assert!(dec(&sealed[..cut]).is_err(), "truncated to {cut}");
        }
    }

    #[test]
    fn detects_reordered_and_appended_chunks() {
        let cs = CS as usize + TAG_LEN;
        let sealed = enc(&data(3 * CS as usize + 1));
        let body = &sealed[HEADER_LEN..];
        let (c0, c1) = (&body[..cs], &body[cs..2 * cs]);
        let swapped = [&sealed[..HEADER_LEN], c1, c0, &body[2 * cs..]].concat();
        assert!(dec(&swapped).is_err());

        // Exactly two full chunks, then garbage appended after the final one.
        let sealed = enc(&data(2 * CS as usize));
        let appended = [&sealed[..], &[0u8; 40][..]].concat();
        assert!(dec(&appended).is_err());
    }

    #[test]
    fn wrong_context_or_key_fails() {
        let sealed = enc(b"attachment");
        let mut out = Vec::new();
        assert!(matches!(
            decrypt(&keys(), b"other", &sealed[..], &mut out),
            Err(CryptoError::Decrypt)
        ));
        let other = Keyring::derive(&SecretKey::from_bytes([4; 32]), 1);
        assert!(matches!(
            decrypt(&other, b"blob", &sealed[..], &mut out),
            Err(CryptoError::Decrypt)
        ));
    }

    #[test]
    fn blob_ids_are_keyed_and_deterministic() {
        let a = blob_id(&keys(), &b"cat.png bytes"[..]).unwrap();
        assert_eq!(a.len(), 64);
        assert_eq!(a, blob_id(&keys(), &b"cat.png bytes"[..]).unwrap());
        assert_ne!(a, blob_id(&keys(), &b"dog.png bytes"[..]).unwrap());
        let other = Keyring::derive(&SecretKey::from_bytes([4; 32]), 1);
        assert_ne!(a, blob_id(&other, &b"cat.png bytes"[..]).unwrap());
    }
}
