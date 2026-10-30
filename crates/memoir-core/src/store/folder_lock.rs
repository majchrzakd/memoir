//! Folder passwords. A folder with a password shows its contents only once
//! the password has been entered since the notebook was unlocked. This is a
//! lock screen, not extra encryption: the contents are protected like the
//! rest of the notebook, and anyone with the notebook passphrase can remove
//! the password. What's stored (and synced with the folder) is an Argon2id
//! hash of the password, never the password.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use serde::{Deserialize, Serialize};

use crate::crypto::random_bytes;
use crate::vault::{KdfParams, derive_kek, normalize};

const KDF_ARGON2ID: &str = "argon2id";
const SALT_LEN: usize = 16;

/// Cheaper than the notebook passphrase: the hash sits inside the encrypted
/// notebook, so guessing it needs the notebook key, which opens the
/// contents anyway. ~30 ms.
const PARAMS: KdfParams = KdfParams {
    m_kib: 19 * 1024,
    t: 2,
    p: 1,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FolderLock {
    kdf: String,
    params: KdfParams,
    salt: String,
    hash: String,
}

impl FolderLock {
    /// None if the password is empty.
    pub fn new(password: &str) -> Option<Self> {
        if password.is_empty() {
            return None;
        }
        let salt = random_bytes::<SALT_LEN>();
        let hash = derive_kek(&normalize(password), &salt, PARAMS).ok()?;
        Some(Self {
            kdf: KDF_ARGON2ID.into(),
            params: PARAMS,
            salt: B64.encode(salt),
            hash: B64.encode(hash.as_bytes()),
        })
    }

    pub fn verify(&self, password: &str) -> bool {
        if self.kdf != KDF_ARGON2ID {
            return false;
        }
        let (Ok(salt), Ok(expected)) = (B64.decode(&self.salt), B64.decode(&self.hash)) else {
            return false;
        };
        let Ok(hash) = derive_kek(&normalize(password), &salt, self.params) else {
            return false;
        };
        let actual = hash.as_bytes();
        // Constant time, so timing reveals nothing about the hash.
        actual.len() == expected.len()
            && actual
                .iter()
                .zip(&expected)
                .fold(0u8, |acc, (a, b)| acc | (a ^ b))
                == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verifies_only_the_right_password() {
        let lock = FolderLock::new("pa55 wörd").unwrap();
        assert!(lock.verify("pa55 wörd"));
        // Decomposed ö, as macOS may type it.
        assert!(lock.verify("pa55 wo\u{308}rd"));
        assert!(!lock.verify("pa55 word"));
        assert!(!lock.verify(""));
        assert!(FolderLock::new("").is_none());
        // Salted: the same password hashes differently each time.
        assert_ne!(FolderLock::new("pa55 wörd").unwrap(), lock);

        let json = serde_json::to_string(&lock).unwrap();
        assert!(!json.contains("pa55"));
        assert!(
            serde_json::from_str::<FolderLock>(&json)
                .unwrap()
                .verify("pa55 wörd")
        );
    }
}
