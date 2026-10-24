//! M0 spike 1: SQLCipher + FTS5 build and behave correctly on this platform.

use memoir_core::db::open_encrypted;
use rusqlite::{Connection, params};

const KEY: [u8; 32] = [7; 32];
const SECRET_WORD: &str = "xylophonic";

fn create_notes(conn: &Connection) {
    conn.execute_batch(
        "CREATE VIRTUAL TABLE notes_fts USING fts5(title, body, tokenize = 'unicode61 remove_diacritics 2');",
    )
    .unwrap();
    conn.execute(
        "INSERT INTO notes_fts(title, body) VALUES (?1, ?2)",
        params!["Shopping", format!("buy a {SECRET_WORD} instrument")],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO notes_fts(title, body) VALUES (?1, ?2)",
        params!["Zażółć", "gęślą jaźń — Polish diacritics"],
    )
    .unwrap();
}

#[test]
fn sqlcipher_is_linked() {
    let dir = tempfile::tempdir().unwrap();
    let conn = open_encrypted(&dir.path().join("t.db"), &KEY).unwrap();
    let version: String = conn
        .query_row("PRAGMA cipher_version", [], |r| r.get(0))
        .unwrap();
    assert!(!version.is_empty());
}

#[test]
fn fts5_search_works_in_encrypted_db() {
    let dir = tempfile::tempdir().unwrap();
    let conn = open_encrypted(&dir.path().join("t.db"), &KEY).unwrap();
    create_notes(&conn);

    let title: String = conn
        .query_row(
            "SELECT title FROM notes_fts WHERE notes_fts MATCH ?1",
            [SECRET_WORD],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(title, "Shopping");

    // Diacritic-insensitive search: "gesla" finds "gęślą".
    let title: String = conn
        .query_row(
            "SELECT title FROM notes_fts WHERE notes_fts MATCH 'gesla'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(title, "Zażółć");

    // Prefix search for search-as-you-type.
    let hits: i64 = conn
        .query_row(
            "SELECT count(*) FROM notes_fts WHERE notes_fts MATCH 'xyl*'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(hits, 1);
}

#[test]
fn file_on_disk_is_ciphertext() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.db");
    {
        let conn = open_encrypted(&path, &KEY).unwrap();
        create_notes(&conn);
        conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
            .unwrap();
    }
    let bytes = std::fs::read(&path).unwrap();
    assert!(bytes.len() > 4096);
    assert!(!bytes.starts_with(b"SQLite format 3"));
    assert!(
        !bytes
            .windows(SECRET_WORD.len())
            .any(|w| w == SECRET_WORD.as_bytes())
    );
}

#[test]
fn wrong_key_is_rejected_and_right_key_reopens() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.db");
    {
        let conn = open_encrypted(&path, &KEY).unwrap();
        create_notes(&conn);
    }

    assert!(open_encrypted(&path, &[8; 32]).is_err());

    let conn = open_encrypted(&path, &KEY).unwrap();
    let n: i64 = conn
        .query_row("SELECT count(*) FROM notes_fts", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 2);
}
