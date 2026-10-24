//! Encrypted local database (SQLCipher).

use std::fmt::Write as _;
use std::path::Path;

use rusqlite::Connection;
use zeroize::Zeroizing;

use crate::crypto::SecretKey;
use crate::crypto::keys::KEY_LEN;

/// Opens (or creates) a SQLCipher database keyed with a raw 256-bit key
/// (normally `Keyring::local_db`). A raw key skips SQLCipher's own PBKDF2.
///
/// A wrong key fails here (with `SQLITE_NOTADB`) rather than on first use.
pub fn open_encrypted(path: &Path, key: &SecretKey) -> rusqlite::Result<Connection> {
    let conn = Connection::open(path)?;
    // SQLCipher otherwise prints decryption failures (e.g. a wrong key) to stderr.
    conn.execute_batch("PRAGMA cipher_log_level = NONE;")?;

    let mut pragma = Zeroizing::new(String::with_capacity(20 + KEY_LEN * 2));
    pragma.push_str("PRAGMA key = \"x'");
    for b in key.as_bytes() {
        let _ = write!(pragma, "{b:02x}");
    }
    pragma.push_str("'\";");
    conn.execute_batch(&pragma)?;

    // SQLCipher only verifies the key when the first page is read.
    conn.query_row("SELECT count(*) FROM sqlite_master", [], |_| Ok(()))?;
    conn.execute_batch("PRAGMA journal_mode = WAL; PRAGMA foreign_keys = ON;")?;
    Ok(conn)
}
