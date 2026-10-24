//! Envelope for synced objects (notes, folders): one encrypted file per
//! object revision.
//!
//! ```text
//! header  = "MMR1" | version u8 | kind u8 (=1) | epoch u32 LE | nonce [24]
//! body    = XChaCha20-Poly1305(object key, nonce, plaintext, aad = header || context)
//! plain   = len u32 LE | zstd(payload) [len] | zero padding to a multiple of 256
//! ```
//!
//! `context` binds the ciphertext to where it belongs (object id and revision
//! id), so a file renamed or swapped by the storage provider fails to open.
//! Padding hides exact note sizes.

use std::io::Read;

use chacha20poly1305::XNonce;
use chacha20poly1305::aead::{Aead, Payload};
use zeroize::Zeroizing;

use super::keys::{NONCE_LEN, TAG_LEN};
use super::{
    CryptoError, FORMAT_VERSION, Keyring, Kind, MAGIC, Result, check_header, random_bytes,
};

pub const HEADER_LEN: usize = 4 + 1 + 1 + 4 + NONCE_LEN;
const PAD_BLOCK: usize = 256;
const ZSTD_LEVEL: i32 = 3;
/// Refuse to inflate anything larger; notes are far smaller.
const MAX_PAYLOAD: u64 = 64 * 1024 * 1024;

/// Encrypts an object payload.
pub fn seal(keys: &Keyring, context: &[u8], payload: &[u8]) -> Vec<u8> {
    seal_with_nonce(keys, random_bytes(), context, payload)
}

pub(crate) fn seal_with_nonce(
    keys: &Keyring,
    nonce: [u8; NONCE_LEN],
    context: &[u8],
    payload: &[u8],
) -> Vec<u8> {
    let mut out = Vec::with_capacity(HEADER_LEN + payload.len() / 2 + PAD_BLOCK + TAG_LEN);
    out.extend_from_slice(MAGIC);
    out.push(FORMAT_VERSION);
    out.push(Kind::Object as u8);
    out.extend_from_slice(&keys.epoch.to_le_bytes());
    out.extend_from_slice(&nonce);

    let plaintext = frame(payload);
    let aad = [&out[..HEADER_LEN], context].concat();
    let ct = keys
        .object
        .cipher()
        .encrypt(
            &XNonce::from(nonce),
            Payload {
                msg: &plaintext,
                aad: &aad,
            },
        )
        .expect("XChaCha20-Poly1305 encryption cannot fail for in-memory data");
    out.extend_from_slice(&ct);
    out
}

/// Decrypts an object; fails if the key, the context or any byte is wrong.
pub fn open(keys: &Keyring, context: &[u8], sealed: &[u8]) -> Result<Vec<u8>> {
    check_header(sealed, Kind::Object, keys)?;
    if sealed.len() < HEADER_LEN + TAG_LEN {
        return Err(CryptoError::Truncated);
    }
    let (header, ct) = sealed.split_at(HEADER_LEN);
    let nonce: [u8; NONCE_LEN] = header[10..].try_into().expect("header ends with nonce");
    let aad = [header, context].concat();
    let plaintext = Zeroizing::new(
        keys.object
            .cipher()
            .decrypt(&XNonce::from(nonce), Payload { msg: ct, aad: &aad })
            .map_err(|_| CryptoError::Decrypt)?,
    );
    unframe(&plaintext)
}

fn frame(payload: &[u8]) -> Zeroizing<Vec<u8>> {
    let compressed = Zeroizing::new(
        zstd::bulk::compress(payload, ZSTD_LEVEL).expect("zstd compression of in-memory data"),
    );
    let len = u32::try_from(compressed.len()).expect("object payloads are far below 4 GiB");
    let padded = (4 + compressed.len()).div_ceil(PAD_BLOCK) * PAD_BLOCK;

    let mut out = Zeroizing::new(Vec::with_capacity(padded));
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(&compressed);
    out.resize(padded, 0);
    out
}

fn unframe(plain: &[u8]) -> Result<Vec<u8>> {
    let len_bytes = plain
        .get(..4)
        .ok_or(CryptoError::Malformed("missing length"))?;
    let len = u32::from_le_bytes(len_bytes.try_into().expect("4 bytes")) as usize;
    let compressed = plain
        .get(4..4 + len)
        .ok_or(CryptoError::Malformed("length exceeds data"))?;
    if plain[4 + len..].iter().any(|&b| b != 0) {
        return Err(CryptoError::Malformed("non-zero padding"));
    }

    let mut payload = Vec::new();
    zstd::stream::read::Decoder::new(compressed)
        .map_err(|_| CryptoError::Malformed("zstd"))?
        .take(MAX_PAYLOAD + 1)
        .read_to_end(&mut payload)
        .map_err(|_| CryptoError::Malformed("zstd"))?;
    if payload.len() as u64 > MAX_PAYLOAD {
        return Err(CryptoError::Malformed("payload too large"));
    }
    Ok(payload)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::SecretKey;
    use crate::crypto::keys::tests::hex;

    fn keys() -> Keyring {
        Keyring::derive(&SecretKey::from_bytes([1; 32]), 1)
    }

    #[test]
    fn roundtrip_various_sizes() {
        let keys = keys();
        for size in [0, 1, 251, 252, 253, 1000, 100_000] {
            let payload: Vec<u8> = (0..size).map(|i| (i * 7 % 251) as u8).collect();
            let sealed = seal(&keys, b"obj:1:rev:1", &payload);
            assert_eq!((sealed.len() - HEADER_LEN - TAG_LEN) % PAD_BLOCK, 0);
            assert_eq!(open(&keys, b"obj:1:rev:1", &sealed).unwrap(), payload);
        }
    }

    #[test]
    fn same_payload_encrypts_differently_each_time() {
        let keys = keys();
        assert_ne!(seal(&keys, b"c", b"same"), seal(&keys, b"c", b"same"));
    }

    #[test]
    fn padding_hides_small_size_differences() {
        let keys = keys();
        let a = seal(&keys, b"c", b"short note");
        let b = seal(&keys, b"c", b"a somewhat longer note, still small");
        assert_eq!(a.len(), b.len());
    }

    #[test]
    fn rejects_wrong_context_key_and_tampering() {
        let keys = keys();
        let sealed = seal(&keys, b"obj:1:rev:1", b"secret");

        // Swapped file (same key, different identity).
        assert!(matches!(
            open(&keys, b"obj:2:rev:1", &sealed),
            Err(CryptoError::Decrypt)
        ));
        // Wrong key.
        let other = Keyring::derive(&SecretKey::from_bytes([2; 32]), 1);
        assert!(matches!(
            open(&other, b"obj:1:rev:1", &sealed),
            Err(CryptoError::Decrypt)
        ));
        // Every single-bit flip is detected (header bytes included).
        for i in 0..sealed.len() {
            let mut t = sealed.clone();
            t[i] ^= 0x01;
            assert!(open(&keys, b"obj:1:rev:1", &t).is_err(), "flip at byte {i}");
        }
        // Truncation.
        assert!(open(&keys, b"obj:1:rev:1", &sealed[..sealed.len() - 1]).is_err());
        assert!(matches!(
            open(&keys, b"obj:1:rev:1", &sealed[..20]),
            Err(CryptoError::Truncated)
        ));
    }

    #[test]
    fn header_errors_are_specific() {
        let keys = keys();
        let sealed = seal(&keys, b"c", b"x");

        let mut bad = sealed.clone();
        bad[0] = b'X';
        assert!(matches!(
            open(&keys, b"c", &bad),
            Err(CryptoError::BadMagic)
        ));

        let mut bad = sealed.clone();
        bad[4] = 9;
        assert!(matches!(
            open(&keys, b"c", &bad),
            Err(CryptoError::UnsupportedVersion(9))
        ));

        let mut bad = sealed.clone();
        bad[5] = Kind::Blob as u8;
        assert!(matches!(
            open(&keys, b"c", &bad),
            Err(CryptoError::WrongKind { .. })
        ));

        let next_epoch = Keyring::derive(&SecretKey::from_bytes([1; 32]), 2);
        assert!(matches!(
            open(&next_epoch, b"c", &sealed),
            Err(CryptoError::UnknownEpoch(1))
        ));
    }

    /// A file written by format version 1. It must stay readable forever:
    /// if this test breaks, files already in users' Drives would too. Bump
    /// FORMAT_VERSION and keep a reader for version 1 instead.
    const V1_FIXTURE: &str = concat!(
        "4d4d5231010101000000090909090909090909090909090909090909090909090909592fc203c96a65df1ae542e9a39d",
        "1a15cc043ffb079138425559daccf80879556cd59496a38cfab7fc05851eca08dfb25e2cb385c0b4ae887f33808320bc",
        "8bfe6df5b5dd851942e612903fc8df48a3fd78d967dda8886984c7178564d4b8a6ecaf9400e16757c4f933c1d290a520",
        "1f4874bf43eab346332c566f6ceebaed8ddf2de6b9e5d393bf59d34017028b5dad97b7cd449856388ead180aac1884b5",
        "202661c8415448518af86e68674526fabb644c778928802dd06f529a0911d8bef1f4e988407681d698d9db69586e5a91",
        "6f55573cc83dd03f6382b97db6a9f809d1ba8aa52bb0767ffbe2b22ffe830ec16cadbf0f3fd5fb5f9c593cb248a645c4",
        "29e654e9e330c9934e5ed82d9db83b5652f9",
    );

    #[test]
    fn version_1_files_stay_readable() {
        let sealed = seal_with_nonce(&keys(), [9; NONCE_LEN], b"ctx", b"hello memoir");
        assert_eq!(
            hex(&sealed[..HEADER_LEN]),
            // magic "MMR1", version 1, kind object, epoch 1, nonce
            "4d4d5231".to_owned() + "01" + "01" + "01000000" + &"09".repeat(24)
        );

        let fixture: Vec<u8> = (0..V1_FIXTURE.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&V1_FIXTURE[i..i + 2], 16).unwrap())
            .collect();
        assert_eq!(open(&keys(), b"ctx", &fixture).unwrap(), b"hello memoir");
    }
}
