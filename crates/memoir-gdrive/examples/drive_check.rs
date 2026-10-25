//! Checks the Drive provider against your real Google Drive.
//!
//! ```powershell
//! $env:MEMOIR_GOOGLE_CLIENT_ID = "....apps.googleusercontent.com"
//! $env:MEMOIR_GOOGLE_CLIENT_SECRET = "..."
//! cargo run -p memoir-gdrive --example drive_check
//! ```
//!
//! Signs in through the browser, then works in a separate "Memoir check"
//! folder (never the real "Memoir" one): uploads a small file and a 12 MB
//! file (resumable upload), lists, downloads, follows the change feed, and
//! deletes everything again. You can remove the empty folder afterwards.

use std::error::Error;
use std::time::{Duration, Instant};

use memoir_core::remote::{Area, StorageProvider};
use memoir_gdrive::{DriveConfig, DriveProvider, OAuthConfig, oauth};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

#[tokio::main]
async fn main() -> Result<()> {
    let client_id = std::env::var("MEMOIR_GOOGLE_CLIENT_ID")
        .map_err(|_| "set MEMOIR_GOOGLE_CLIENT_ID (see docs/spikes/google-oauth.md)")?;
    let client_secret = std::env::var("MEMOIR_GOOGLE_CLIENT_SECRET").unwrap_or_default();
    let oauth_config = OAuthConfig::google(client_id, client_secret);
    let http = reqwest::Client::new();

    println!("[1] Signing in — a browser tab opens…");
    let sign_in = oauth::sign_in(
        &oauth_config,
        &http,
        |url| open::that(url).map_err(|e| e.to_string()),
        std::future::pending(),
    )
    .await?;
    println!("    ok");

    let mut config = DriveConfig::google(oauth_config);
    config.root_folder = "Memoir check".into();
    let drive = DriveProvider::new(config, sign_in.refresh_token);

    let account = drive.account().await?;
    println!("[2] Signed in as {} ({})", account.email, account.name);
    let folders = drive.ensure_folders().await?;
    let again = drive.ensure_folders().await?;
    assert_eq!(folders, again, "folders must be found, not created twice");
    println!("[3] Folders ready (found again on second call)");

    let cursor = drive.cursor().await?;
    let small = b"MMR1 small encrypted object".to_vec();
    let big: Vec<u8> = (0..12 * 1024 * 1024).map(|i| (i % 251) as u8).collect();
    let started = Instant::now();
    let small_file = drive
        .upload(Area::Objects, "check.r1.mmr", small.clone())
        .await?;
    let big_file = drive
        .upload(Area::Blobs, "check-big.mmr", big.clone())
        .await?;
    println!(
        "[4] Uploaded {} B + {} MB (resumable) in {:.1}s",
        small.len(),
        big.len() / 1024 / 1024,
        started.elapsed().as_secs_f64()
    );

    let objects = drive.list(Area::Objects).await?;
    let blobs = drive.list(Area::Blobs).await?;
    assert!(objects.iter().any(|f| f.id == small_file.id));
    assert!(
        blobs
            .iter()
            .any(|f| f.id == big_file.id && f.size == big.len() as u64)
    );
    println!("[5] Listed: both files present, sizes match");

    assert_eq!(drive.download(&small_file.id).await?, small);
    assert_eq!(drive.download(&big_file.id).await?, big);
    println!("[6] Downloaded both: contents match byte for byte");

    let changes = drive.changes(&cursor).await?;
    assert!(changes.files.iter().any(|f| f.id == small_file.id));
    assert!(changes.files.iter().any(|f| f.id == big_file.id));
    println!("[7] Change feed reports both new files");

    drive.delete(&small_file.id).await?;
    drive.delete(&big_file.id).await?;
    let deleted_at = Instant::now();
    let objects = drive.list(Area::Objects).await?;
    let blobs = drive.list(Area::Blobs).await?;
    assert!(!objects.iter().any(|f| f.id == small_file.id));
    assert!(!blobs.iter().any(|f| f.id == big_file.id));
    println!("[8] Deleted both; listing confirms they're gone");

    // The change feed may lag behind; measure how long deletions take to show.
    let mut cursor = changes.cursor;
    let (mut seen_small, mut seen_big) = (false, false);
    let mut polls = 0;
    while !(seen_small && seen_big) && deleted_at.elapsed() < Duration::from_secs(60) {
        let after = drive.changes(&cursor).await?;
        polls += 1;
        if !after.files.is_empty() || !after.removed.is_empty() {
            println!(
                "    poll {polls} at {:.1}s: {} file(s), {} removal(s)",
                deleted_at.elapsed().as_secs_f64(),
                after.files.len(),
                after.removed.len()
            );
        }
        seen_small |= after.removed.contains(&small_file.id);
        seen_big |= after.removed.contains(&big_file.id);
        cursor = after.cursor;
        if !(seen_small && seen_big) {
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    }
    if seen_small && seen_big {
        println!(
            "[9] Change feed reported both deletions after {:.1}s ({polls} poll(s))",
            deleted_at.elapsed().as_secs_f64()
        );
    } else {
        println!(
            "[9] FINDING: after 60s the change feed reported deletion of small: {seen_small}, \
             big: {seen_big}"
        );
    }

    drive.revoke().await?;
    println!("\nDone. Access was revoked again; you can delete the empty");
    println!("\"Memoir check\" folder from your Drive.");
    Ok(())
}
