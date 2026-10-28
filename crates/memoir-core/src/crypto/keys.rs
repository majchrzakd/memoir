//! Key types and the derivation of purpose-specific keys from the master key.

use std::fmt;

use chacha20poly1305::aead::Aead;
use chacha20poly1305::{KeyInit, XChaCha20Poly1305, XNonce};
use hkdf::Hkdf;
use sha2::Sha256;
use zeroize::{Zeroize, ZeroizeOnDrop};

use super::{CryptoError, random_bytes};

pub const KEY_LEN: usize = 32;
pub const NONCE_LEN: usize = 24;
pub const TAG_LEN: usize = 16;

/// A 256-bit secret key. Wiped from memory on drop and never printed.
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct SecretKey([u8; KEY_LEN]);

impl SecretKey {
    pub fn generate() -> Self {
        Self(random_bytes())
    }

    pub fn from_bytes(bytes: [u8; KEY_LEN]) -> Self {
        Self(bytes)
    }

    pub fn from_slice(bytes: &[u8]) -> Option<Self> {
        Some(Self(bytes.try_into().ok()?))
    }

    pub fn as_bytes(&self) -> &[u8; KEY_LEN] {
        &self.0
    }

    /// Derives a subkey with HKDF-SHA256. `info` must be a unique label.
    pub fn derive(&self, info: &[u8]) -> SecretKey {
        let mut out = [0u8; KEY_LEN];
        Hkdf::<Sha256>::new(None, &self.0)
            .expand(info, &mut out)
            .expect("32 bytes is a valid HKDF-SHA256 output length");
        let key = SecretKey(out);
        out.zeroize();
        key
    }

    pub(crate) fn cipher(&self) -> XChaCha20Poly1305 {
        XChaCha20Poly1305::new(&self.0.into())
    }

    /// Encrypts `plaintext` under this key; returns nonce || ciphertext || tag.
    pub(crate) fn seal(&self, aad: &[u8], plaintext: &[u8]) -> Vec<u8> {
        let nonce: [u8; NONCE_LEN] = random_bytes();
        let ct = self
            .cipher()
            .encrypt(
                &XNonce::from(nonce),
                chacha20poly1305::aead::Payload {
                    msg: plaintext,
                    aad,
                },
            )
            .expect("XChaCha20-Poly1305 encryption cannot fail for in-memory data");
        let mut out = Vec::with_capacity(NONCE_LEN + ct.len());
        out.extend_from_slice(&nonce);
        out.extend_from_slice(&ct);
        out
    }

    /// Inverse of [`SecretKey::seal`].
    pub(crate) fn open(&self, aad: &[u8], sealed: &[u8]) -> Result<Vec<u8>, CryptoError> {
        if sealed.len() < NONCE_LEN + TAG_LEN {
            return Err(CryptoError::Truncated);
        }
        let (nonce, ct) = sealed.split_at(NONCE_LEN);
        let nonce: [u8; NONCE_LEN] = nonce.try_into().expect("split at NONCE_LEN");
        self.cipher()
            .decrypt(
                &XNonce::from(nonce),
                chacha20poly1305::aead::Payload { msg: ct, aad },
            )
            .map_err(|_| CryptoError::Decrypt)
    }
}

impl fmt::Debug for SecretKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretKey(<redacted>)")
    }
}

/// Purpose-specific keys derived from the vault's master key. Using a
/// separate key per purpose means a bug in one use can't leak another.
#[derive(Debug, Clone)]
pub struct Keyring {
    /// Key generation; recorded in every encrypted file's header.
    pub epoch: u32,
    /// Encrypts notes and folders.
    pub object: SecretKey,
    /// Encrypts attachments.
    pub blob: SecretKey,
    /// Keys the HMAC that names attachments.
    pub blob_id: SecretKey,
    /// SQLCipher key for the local database.
    pub local_db: SecretKey,
    /// Signs `vault.json` changes, so other devices accept a new passphrase
    /// or recovery key only from someone holding the master key.
    pub vault_mac: SecretKey,
}

impl Keyring {
    pub fn derive(master: &SecretKey, epoch: u32) -> Self {
        Self {
            epoch,
            object: master.derive(b"memoir/v1/object"),
            blob: master.derive(b"memoir/v1/blob"),
            blob_id: master.derive(b"memoir/v1/blob-id"),
            local_db: master.derive(b"memoir/v1/local-db"),
            vault_mac: master.derive(b"memoir/v1/vault-mac"),
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    #[test]
    fn debug_does_not_leak_key_material() {
        let key = SecretKey::from_bytes([0xAB; KEY_LEN]);
        let printed = format!("{key:?} {:?}", Keyring::derive(&key, 1));
        assert!(!printed.to_lowercase().contains("ab, ab"));
        assert!(!printed.contains("171"));
        assert!(printed.contains("redacted"));
    }

    #[test]
    fn subkeys_are_distinct_and_deterministic() {
        let master = SecretKey::from_bytes([1; KEY_LEN]);
        let a = Keyring::derive(&master, 1);
        let b = Keyring::derive(&master, 1);
        assert_eq!(a.object.as_bytes(), b.object.as_bytes());

        let all = [&a.object, &a.blob, &a.blob_id, &a.local_db, &a.vault_mac];
        for (i, x) in all.iter().enumerate() {
            assert_ne!(x.as_bytes(), master.as_bytes());
            for y in &all[i + 1..] {
                assert_ne!(x.as_bytes(), y.as_bytes());
            }
        }
    }

    #[test]
    fn subkey_derivation_is_stable() {
        // Regression vector: changing derivation would make existing vaults
        // unreadable. Generated by this implementation (HKDF-SHA256).
        let master = SecretKey::from_bytes([1; KEY_LEN]);
        let keys = Keyring::derive(&master, 1);
        assert_eq!(
            hex(keys.object.as_bytes()),
            "0edf653ebf1e00f1d109696233355347120e5a38c5828571b2bb4908ba7a834a"
        );
    }

    #[test]
    fn seal_open_roundtrip_and_tamper_detection() {
        let key = SecretKey::generate();
        let sealed = key.seal(b"aad", b"hello");
        assert_eq!(key.open(b"aad", &sealed).unwrap(), b"hello");
        assert!(matches!(
            key.open(b"other", &sealed),
            Err(CryptoError::Decrypt)
        ));
        assert!(matches!(
            SecretKey::generate().open(b"aad", &sealed),
            Err(CryptoError::Decrypt)
        ));
        let mut flipped = sealed.clone();
        *flipped.last_mut().unwrap() ^= 1;
        assert!(matches!(
            key.open(b"aad", &flipped),
            Err(CryptoError::Decrypt)
        ));
        assert!(matches!(
            key.open(b"aad", &sealed[..20]),
            Err(CryptoError::Truncated)
        ));
    }

    pub(crate) fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }
}
