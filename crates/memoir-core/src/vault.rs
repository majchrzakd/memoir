//! The vault: a random master key, wrapped once by the user's passphrase
//! (Argon2id) and once by the recovery key, stored as `vault.json` next to
//! the encrypted data. The file holds no secret in the clear; without the
//! passphrase or the recovery key it is useless.
//!
//! Changing the passphrase or the recovery key only re-wraps the master key,
//! so it is instant and no data has to be re-encrypted.

use argon2::{Algorithm, Argon2, Params, Version};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use serde::{Deserialize, Serialize};
use unicode_normalization::UnicodeNormalization;
use zeroize::Zeroizing;

use crate::crypto::keys::KEY_LEN;
use crate::crypto::{Keyring, RecoveryKey, SecretKey, random_bytes};

pub const VAULT_FORMAT: &str = "memoir-vault";
pub const VAULT_VERSION: u32 = 1;
pub const MIN_PASSPHRASE_CHARS: usize = 8;
const KDF_ARGON2ID: &str = "argon2id";
const KDF_HKDF: &str = "hkdf-sha256";
const SALT_LEN: usize = 16;

#[derive(Debug, thiserror::Error)]
pub enum VaultError {
    #[error("wrong passphrase")]
    WrongPassphrase,
    #[error("wrong recovery key")]
    WrongRecoveryKey,
    #[error("the passphrase must be at least {MIN_PASSPHRASE_CHARS} characters")]
    PassphraseTooShort,
    #[error("vault.json is not a valid Memoir vault: {0}")]
    Malformed(String),
    #[error("this vault was created by a newer version of Memoir (format {0})")]
    UnsupportedVersion(u32),
    #[error("invalid key-derivation parameters")]
    InvalidKdfParams,
}

pub type Result<T> = std::result::Result<T, VaultError>;

/// Argon2id cost parameters, stored in the vault so they can be raised later.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct KdfParams {
    /// Memory in KiB.
    pub m_kib: u32,
    /// Passes.
    pub t: u32,
    /// Lanes.
    pub p: u32,
}

impl KdfParams {
    /// 256 MiB, 3 passes, 1 lane. Unlocking with the passphrase is rare (new
    /// device, or "require passphrase on launch"), so it can afford to be
    /// expensive for an attacker: ~0.3 s on a current desktop.
    pub const DEFAULT: Self = Self {
        m_kib: 256 * 1024,
        t: 3,
        p: 1,
    };

    /// Bounds checked before hashing, so a tampered vault.json can't make
    /// the app allocate gigabytes or spin for minutes.
    fn validate(self) -> Result<Params> {
        if self.m_kib > 1024 * 1024 || self.t == 0 || self.t > 16 || self.p == 0 || self.p > 8 {
            return Err(VaultError::InvalidKdfParams);
        }
        Params::new(self.m_kib, self.t, self.p, Some(KEY_LEN))
            .map_err(|_| VaultError::InvalidKdfParams)
    }
}

/// Contents of `vault.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VaultFile {
    pub format: String,
    pub version: u32,
    /// Random, public identifier of this vault.
    pub vault_id: String,
    /// Key generation; matches the epoch in every encrypted file header.
    pub key_epoch: u32,
    pub passphrase: PassphraseSlot,
    pub recovery: RecoverySlot,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PassphraseSlot {
    pub kdf: String,
    #[serde(flatten)]
    pub params: KdfParams,
    pub salt: String,
    /// base64url(nonce || XChaCha20-Poly1305(master key)).
    pub wrapped_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoverySlot {
    pub kdf: String,
    pub wrapped_key: String,
}

/// Result of creating a vault. Show `recovery_key` to the user exactly once.
#[derive(Debug)]
pub struct NewVault {
    pub file: VaultFile,
    pub vault: UnlockedVault,
    pub recovery_key: RecoveryKey,
}

/// The master key and its derived keys, in memory only.
#[derive(Debug)]
pub struct UnlockedVault {
    vault_id: String,
    master: SecretKey,
    keys: Keyring,
}

pub fn create(passphrase: &str, params: KdfParams) -> Result<NewVault> {
    let passphrase = new_passphrase(passphrase)?;
    params.validate()?;

    let master = SecretKey::generate();
    let recovery_key = RecoveryKey::generate();
    let vault_id = B64.encode(random_bytes::<16>());
    let key_epoch = 1;
    let file = VaultFile {
        format: VAULT_FORMAT.into(),
        version: VAULT_VERSION,
        passphrase: passphrase_slot(&master, &vault_id, key_epoch, &passphrase, params)?,
        recovery: recovery_slot(&master, &vault_id, key_epoch, &recovery_key),
        vault_id,
        key_epoch,
    };
    let vault = UnlockedVault::from_master(&file, master);
    Ok(NewVault {
        file,
        vault,
        recovery_key,
    })
}

impl VaultFile {
    pub fn from_json(json: &str) -> Result<Self> {
        let file: Self =
            serde_json::from_str(json).map_err(|e| VaultError::Malformed(e.to_string()))?;
        if file.format != VAULT_FORMAT {
            return Err(VaultError::Malformed(format!(
                "format is {:?}",
                file.format
            )));
        }
        if file.version > VAULT_VERSION {
            return Err(VaultError::UnsupportedVersion(file.version));
        }
        if file.passphrase.kdf != KDF_ARGON2ID || file.recovery.kdf != KDF_HKDF {
            return Err(VaultError::Malformed(
                "unknown key-derivation function".into(),
            ));
        }
        file.passphrase.params.validate()?;
        Ok(file)
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("vault file serializes")
    }

    pub fn unlock(&self, passphrase: &str) -> Result<UnlockedVault> {
        let slot = &self.passphrase;
        let salt = decode(&slot.salt)?;
        let kek = derive_kek(&normalize(passphrase), &salt, slot.params)?;
        let aad = passphrase_aad(&self.vault_id, self.key_epoch, slot.params, &slot.salt);
        let master =
            unwrap_key(&kek, &aad, &slot.wrapped_key).ok_or(VaultError::WrongPassphrase)?;
        Ok(UnlockedVault::from_master(self, master))
    }

    pub fn unlock_with_recovery_key(&self, recovery_key: &RecoveryKey) -> Result<UnlockedVault> {
        let aad = recovery_aad(&self.vault_id, self.key_epoch);
        let master = unwrap_key(
            &recovery_key.wrapping_key(),
            &aad,
            &self.recovery.wrapped_key,
        )
        .ok_or(VaultError::WrongRecoveryKey)?;
        Ok(UnlockedVault::from_master(self, master))
    }
}

impl UnlockedVault {
    pub(crate) fn from_master(file: &VaultFile, master: SecretKey) -> Self {
        Self {
            vault_id: file.vault_id.clone(),
            keys: Keyring::derive(&master, file.key_epoch),
            master,
        }
    }

    pub fn vault_id(&self) -> &str {
        &self.vault_id
    }

    pub fn keys(&self) -> &Keyring {
        &self.keys
    }

    pub(crate) fn master(&self) -> &SecretKey {
        &self.master
    }

    /// Returns an updated vault file; the recovery key keeps working.
    pub fn change_passphrase(
        &self,
        file: &VaultFile,
        new_passphrase: &str,
        params: KdfParams,
    ) -> Result<VaultFile> {
        let passphrase = new_passphrase_checked(new_passphrase, params)?;
        Ok(VaultFile {
            passphrase: passphrase_slot(
                &self.master,
                &file.vault_id,
                file.key_epoch,
                &passphrase,
                params,
            )?,
            ..file.clone()
        })
    }

    /// Issues a new recovery key and invalidates the old one; the passphrase
    /// keeps working.
    pub fn replace_recovery_key(&self, file: &VaultFile) -> (VaultFile, RecoveryKey) {
        let recovery_key = RecoveryKey::generate();
        let file = VaultFile {
            recovery: recovery_slot(&self.master, &file.vault_id, file.key_epoch, &recovery_key),
            ..file.clone()
        };
        (file, recovery_key)
    }
}

fn new_passphrase_checked(passphrase: &str, params: KdfParams) -> Result<Zeroizing<String>> {
    params.validate()?;
    new_passphrase(passphrase)
}

fn new_passphrase(passphrase: &str) -> Result<Zeroizing<String>> {
    let normalized = normalize(passphrase);
    if normalized.chars().count() < MIN_PASSPHRASE_CHARS {
        return Err(VaultError::PassphraseTooShort);
    }
    Ok(normalized)
}

/// NFC-normalizes so the same passphrase typed on different systems (macOS
/// tends to produce decomposed accents) derives the same key.
fn normalize(passphrase: &str) -> Zeroizing<String> {
    Zeroizing::new(passphrase.nfc().collect())
}

fn derive_kek(passphrase: &str, salt: &[u8], params: KdfParams) -> Result<SecretKey> {
    let mut out = Zeroizing::new([0u8; KEY_LEN]);
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params.validate()?)
        .hash_password_into(passphrase.as_bytes(), salt, &mut out[..])
        .map_err(|_| VaultError::InvalidKdfParams)?;
    Ok(SecretKey::from_bytes(*out))
}

fn passphrase_slot(
    master: &SecretKey,
    vault_id: &str,
    epoch: u32,
    passphrase: &str,
    params: KdfParams,
) -> Result<PassphraseSlot> {
    let salt = random_bytes::<SALT_LEN>();
    let salt_b64 = B64.encode(salt);
    let kek = derive_kek(passphrase, &salt, params)?;
    let aad = passphrase_aad(vault_id, epoch, params, &salt_b64);
    Ok(PassphraseSlot {
        kdf: KDF_ARGON2ID.into(),
        params,
        salt: salt_b64,
        wrapped_key: B64.encode(kek.seal(&aad, master.as_bytes())),
    })
}

fn recovery_slot(
    master: &SecretKey,
    vault_id: &str,
    epoch: u32,
    recovery_key: &RecoveryKey,
) -> RecoverySlot {
    let aad = recovery_aad(vault_id, epoch);
    RecoverySlot {
        kdf: KDF_HKDF.into(),
        wrapped_key: B64.encode(recovery_key.wrapping_key().seal(&aad, master.as_bytes())),
    }
}

/// Binds a wrapped key to its vault and parameters: editing any of them in
/// vault.json makes unlocking fail instead of silently misbehaving.
fn passphrase_aad(vault_id: &str, epoch: u32, p: KdfParams, salt_b64: &str) -> Vec<u8> {
    format!(
        "memoir/v1/wrap/passphrase|{vault_id}|{epoch}|{KDF_ARGON2ID}|{}|{}|{}|{salt_b64}",
        p.m_kib, p.t, p.p
    )
    .into_bytes()
}

fn recovery_aad(vault_id: &str, epoch: u32) -> Vec<u8> {
    format!("memoir/v1/wrap/recovery|{vault_id}|{epoch}|{KDF_HKDF}").into_bytes()
}

fn unwrap_key(kek: &SecretKey, aad: &[u8], wrapped_b64: &str) -> Option<SecretKey> {
    let wrapped = B64.decode(wrapped_b64).ok()?;
    let plain = Zeroizing::new(kek.open(aad, &wrapped).ok()?);
    SecretKey::from_slice(&plain)
}

fn decode(b64: &str) -> Result<Vec<u8>> {
    B64.decode(b64)
        .map_err(|_| VaultError::Malformed("invalid base64".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::keys::tests::hex;

    /// Cheap parameters so tests run fast; production uses `KdfParams::DEFAULT`.
    const FAST: KdfParams = KdfParams {
        m_kib: 64,
        t: 1,
        p: 1,
    };
    const PASS: &str = "correct horse battery";

    fn same_keys(a: &UnlockedVault, b: &UnlockedVault) -> bool {
        a.keys().object.as_bytes() == b.keys().object.as_bytes()
            && a.keys().local_db.as_bytes() == b.keys().local_db.as_bytes()
    }

    #[test]
    fn create_then_unlock_with_passphrase_or_recovery_key() {
        let new = create(PASS, FAST).unwrap();
        let file = VaultFile::from_json(&new.file.to_json()).unwrap();
        assert_eq!(file, new.file);

        assert!(same_keys(&file.unlock(PASS).unwrap(), &new.vault));
        let rk = RecoveryKey::parse(&new.recovery_key.to_text()).unwrap();
        assert!(same_keys(
            &file.unlock_with_recovery_key(&rk).unwrap(),
            &new.vault
        ));
    }

    #[test]
    fn wrong_secrets_are_rejected() {
        let new = create(PASS, FAST).unwrap();
        assert!(matches!(
            new.file.unlock("correct horse battery!"),
            Err(VaultError::WrongPassphrase)
        ));
        assert!(matches!(
            new.file.unlock(""),
            Err(VaultError::WrongPassphrase)
        ));
        assert!(matches!(
            new.file.unlock_with_recovery_key(&RecoveryKey::generate()),
            Err(VaultError::WrongRecoveryKey)
        ));
    }

    #[test]
    fn passphrase_unicode_normalization() {
        let composed = "zażółć gęślą jaźń";
        let decomposed: String = composed.nfd().collect();
        assert_ne!(composed, decomposed);
        let new = create(composed, FAST).unwrap();
        assert!(new.file.unlock(&decomposed).is_ok());
    }

    #[test]
    fn short_passphrase_is_refused() {
        assert!(matches!(
            create("1234567", FAST),
            Err(VaultError::PassphraseTooShort)
        ));
        // Counted in characters, not bytes.
        assert!(matches!(
            create("ąęółżźć", FAST),
            Err(VaultError::PassphraseTooShort)
        ));
        assert!(create("ąęółżźćń", FAST).is_ok());
    }

    #[test]
    fn tampering_with_vault_file_is_detected() {
        let new = create(PASS, FAST).unwrap();

        let mut f = new.file.clone();
        f.passphrase.params.t = 2;
        assert!(matches!(f.unlock(PASS), Err(VaultError::WrongPassphrase)));

        let mut f = new.file.clone();
        f.vault_id = B64.encode([0u8; 16]);
        assert!(f.unlock(PASS).is_err());
        assert!(f.unlock_with_recovery_key(&new.recovery_key).is_err());

        let mut f = new.file.clone();
        f.key_epoch = 2;
        assert!(f.unlock(PASS).is_err());

        // Swapping in another vault's wrapped key.
        let other = create(PASS, FAST).unwrap();
        let mut f = new.file.clone();
        f.recovery = other.file.recovery.clone();
        assert!(f.unlock_with_recovery_key(&other.recovery_key).is_err());
    }

    #[test]
    fn from_json_validates_before_doing_work() {
        let new = create(PASS, FAST).unwrap();
        let json = new.file.to_json();

        let newer = json.replace("\"version\": 1", "\"version\": 2");
        assert!(matches!(
            VaultFile::from_json(&newer),
            Err(VaultError::UnsupportedVersion(2))
        ));

        let huge = json.replace("\"m_kib\": 64", "\"m_kib\": 4294967295");
        assert!(matches!(
            VaultFile::from_json(&huge),
            Err(VaultError::InvalidKdfParams)
        ));

        let foreign = json.replace(VAULT_FORMAT, "something-else");
        assert!(matches!(
            VaultFile::from_json(&foreign),
            Err(VaultError::Malformed(_))
        ));

        assert!(matches!(
            VaultFile::from_json("{}"),
            Err(VaultError::Malformed(_))
        ));
    }

    #[test]
    fn change_passphrase_keeps_keys_and_recovery() {
        let new = create(PASS, FAST).unwrap();
        let changed = new
            .vault
            .change_passphrase(&new.file, "a brand new passphrase", FAST)
            .unwrap();

        assert!(matches!(
            changed.unlock(PASS),
            Err(VaultError::WrongPassphrase)
        ));
        assert!(same_keys(
            &changed.unlock("a brand new passphrase").unwrap(),
            &new.vault
        ));
        assert!(changed.unlock_with_recovery_key(&new.recovery_key).is_ok());
        assert_ne!(changed.passphrase.salt, new.file.passphrase.salt);
        assert!(matches!(
            new.vault.change_passphrase(&new.file, "short", FAST),
            Err(VaultError::PassphraseTooShort)
        ));
    }

    #[test]
    fn replace_recovery_key_invalidates_the_old_one() {
        let new = create(PASS, FAST).unwrap();
        let (file, rk2) = new.vault.replace_recovery_key(&new.file);
        assert!(file.unlock_with_recovery_key(&new.recovery_key).is_err());
        assert!(same_keys(
            &file.unlock_with_recovery_key(&rk2).unwrap(),
            &new.vault
        ));
        assert!(file.unlock(PASS).is_ok());
    }

    #[test]
    fn argon2id_derivation_is_stable() {
        // Regression vector: a change here would lock users out of their vaults.
        let kek = derive_kek("correct horse battery", &[7; SALT_LEN], FAST).unwrap();
        assert_eq!(
            hex(kek.as_bytes()),
            "562c92fc17ccb36c5048c3d45fe6745b4706422b3098c88099718e09412a1393"
        );
    }

    /// Run manually: `cargo test -p memoir-core --release -- --ignored --nocapture`
    #[test]
    #[ignore = "timing check for production parameters"]
    fn default_params_unlock_time() {
        let new = create(PASS, KdfParams::DEFAULT).unwrap();
        let started = std::time::Instant::now();
        new.file.unlock(PASS).unwrap();
        println!(
            "unlock with default Argon2id params: {:?}",
            started.elapsed()
        );
    }
}
