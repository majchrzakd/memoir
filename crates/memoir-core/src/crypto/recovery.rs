//! The recovery key: a random 256-bit key shown to the user once, as text
//! they can write down, that can unlock the vault if the passphrase is lost.
//!
//! Text form: Crockford base32 of `key || first 2 bytes of SHA-256(key)`,
//! 55 characters in 11 groups of 5, e.g. `7K3QF-…`. Parsing ignores case,
//! spaces and dashes, accepts `I`/`L` for `1` and `O` for `0`, and the
//! checksum catches typos before they look like a wrong key.

use std::fmt;

use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use super::SecretKey;
use super::keys::KEY_LEN;

const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
const CHECKSUM_LEN: usize = 2;
const PAYLOAD_LEN: usize = KEY_LEN + CHECKSUM_LEN;
/// ceil(34 bytes * 8 / 5 bits)
const TEXT_LEN: usize = 55;
const GROUP: usize = 5;

#[derive(Clone)]
pub struct RecoveryKey(SecretKey);

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum RecoveryKeyError {
    #[error("a recovery key has {TEXT_LEN} characters; this one has {0}")]
    WrongLength(usize),
    #[error("'{0}' is not a valid recovery key character")]
    InvalidCharacter(char),
    #[error("this recovery key has a typo (checksum mismatch)")]
    Checksum,
}

impl RecoveryKey {
    pub fn generate() -> Self {
        Self(SecretKey::generate())
    }

    /// The key that wraps the master key in the vault's recovery slot.
    /// No password hashing is needed: the recovery key is already 256 random bits.
    pub(crate) fn wrapping_key(&self) -> SecretKey {
        self.0.derive(b"memoir/v1/recovery-kek")
    }

    /// Text to show the user, grouped for readability.
    pub fn to_text(&self) -> Zeroizing<String> {
        let mut payload = Zeroizing::new([0u8; PAYLOAD_LEN]);
        payload[..KEY_LEN].copy_from_slice(self.0.as_bytes());
        payload[KEY_LEN..].copy_from_slice(&checksum(self.0.as_bytes()));

        let mut out = Zeroizing::new(String::with_capacity(TEXT_LEN + TEXT_LEN / GROUP));
        for i in 0..TEXT_LEN {
            if i > 0 && i % GROUP == 0 {
                out.push('-');
            }
            out.push(ALPHABET[read_5_bits(&payload[..], i * 5) as usize] as char);
        }
        out
    }

    pub fn parse(text: &str) -> Result<Self, RecoveryKeyError> {
        let mut symbols = Zeroizing::new(Vec::with_capacity(TEXT_LEN));
        for c in text.chars() {
            if c == '-' || c.is_whitespace() {
                continue;
            }
            symbols.push(decode_symbol(c).ok_or(RecoveryKeyError::InvalidCharacter(c))?);
        }
        if symbols.len() != TEXT_LEN {
            return Err(RecoveryKeyError::WrongLength(symbols.len()));
        }

        let mut payload = Zeroizing::new([0u8; PAYLOAD_LEN + 1]);
        for (i, &s) in symbols.iter().enumerate() {
            write_5_bits(&mut payload[..], i * 5, s);
        }
        // 55 symbols carry 275 bits; the 3 bits past the payload must be zero.
        if payload[PAYLOAD_LEN] != 0 {
            return Err(RecoveryKeyError::Checksum);
        }
        let key = SecretKey::from_slice(&payload[..KEY_LEN]).expect("KEY_LEN bytes");
        if checksum(key.as_bytes()) != payload[KEY_LEN..PAYLOAD_LEN] {
            return Err(RecoveryKeyError::Checksum);
        }
        Ok(Self(key))
    }
}

impl fmt::Debug for RecoveryKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RecoveryKey(<redacted>)")
    }
}

fn checksum(key: &[u8; KEY_LEN]) -> [u8; CHECKSUM_LEN] {
    let digest = Sha256::digest(key);
    [digest[0], digest[1]]
}

fn decode_symbol(c: char) -> Option<u8> {
    let c = match c.to_ascii_uppercase() {
        'O' => '0',
        'I' | 'L' => '1',
        c => c,
    };
    ALPHABET
        .iter()
        .position(|&a| a as char == c)
        .map(|p| p as u8)
}

/// Reads 5 bits starting at bit `pos` (big-endian), zero-padded past the end.
fn read_5_bits(bytes: &[u8], pos: usize) -> u8 {
    (0..5).fold(0u8, |acc, i| {
        let bit = pos + i;
        let b = bytes
            .get(bit / 8)
            .map_or(0, |byte| (byte >> (7 - bit % 8)) & 1);
        (acc << 1) | b
    })
}

fn write_5_bits(bytes: &mut [u8], pos: usize, value: u8) {
    for i in 0..5 {
        let bit = pos + i;
        if (value >> (4 - i)) & 1 == 1 {
            bytes[bit / 8] |= 1 << (7 - bit % 8);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_roundtrip() {
        for _ in 0..200 {
            let key = RecoveryKey::generate();
            let text = key.to_text();
            assert_eq!(text.len(), TEXT_LEN + TEXT_LEN / GROUP - 1);
            let parsed = RecoveryKey::parse(&text).unwrap();
            assert_eq!(parsed.0.as_bytes(), key.0.as_bytes());
        }
    }

    #[test]
    fn parsing_is_forgiving_about_formatting() {
        let key = RecoveryKey::generate();
        let text = key.to_text();
        let sloppy = format!("  {}  ", text.to_lowercase().replace('-', " "));
        let sloppy = sloppy.replace('0', "o").replace('1', "l");
        let parsed = RecoveryKey::parse(&sloppy).unwrap();
        assert_eq!(parsed.0.as_bytes(), key.0.as_bytes());
    }

    #[test]
    fn typos_are_detected() {
        let text = RecoveryKey::generate().to_text().to_string();
        // Change one character to every other symbol.
        let first = text.chars().next().unwrap();
        for &alt in ALPHABET.iter().filter(|&&a| a as char != first) {
            let typo = format!("{}{}", alt as char, &text[1..]);
            assert_eq!(
                RecoveryKey::parse(&typo).unwrap_err(),
                RecoveryKeyError::Checksum
            );
        }
        assert_eq!(
            RecoveryKey::parse(&text[..text.len() - 1]).unwrap_err(),
            RecoveryKeyError::WrongLength(TEXT_LEN - 1)
        );
        assert_eq!(
            RecoveryKey::parse(&text.replacen(first, "U", 1)).unwrap_err(),
            RecoveryKeyError::InvalidCharacter('U')
        );
    }

    #[test]
    fn known_encoding() {
        let key = RecoveryKey(SecretKey::from_bytes([0; KEY_LEN]));
        // SHA-256 of 32 zero bytes starts with 0x66 0x68, so the last group
        // encodes bits 0 | 0110 0110 0110 1000 | 000 → 00110 01100 11010 00000
        // preceded by one zero symbol: "0", "6", "C", "T", "0".
        assert_eq!(
            key.to_text().as_str(),
            "00000-00000-00000-00000-00000-00000-00000-00000-00000-00000-06CT0"
        );
    }
}
