//! Encrypted local database (SQLCipher).

use std::fmt::Write as _;
use std::path::Path;

use rusqlite::Connection;
use zeroize::Zeroizing;

pub const KEY_LEN: usize = 32;

/// Opens (or creates) a SQLCipher database keyed with a raw 256-bit key.
///
/// A wrong key fails here (with `SQLITE_NOTADB`) rather than on first use.
pub fn open_encrypted(path: &Path, key: &[u8; KEY_LEN]) -> rusqlite::Result<Connection> {
    let conn = Connection::open(path)?;

    let mut pragma = Zeroizing::new(String::with_capacity(20 + KEY_LEN * 2));
    pragma.push_str("PRAGMA key = \"x'");
    for b in key {
        let _ = write!(pragma, "{b:02x}");
    }
    pragma.push_str("'\";");
    conn.execute_batch(&pragma)?;

    // SQLCipher only verifies the key when the first page is read.
    conn.query_row("SELECT count(*) FROM sqlite_master", [], |_| Ok(()))?;
    conn.execute_batch("PRAGMA journal_mode = WAL; PRAGMA foreign_keys = ON;")?;
    Ok(conn)
}
