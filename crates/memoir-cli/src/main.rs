//! `memoir` — offline recovery tool: gets your notes out of a copy of the
//! Memoir folder from Google Drive, without the app.

mod decrypt;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use zeroize::Zeroizing;

use crate::decrypt::{Secret, decrypt};

#[derive(Parser)]
#[command(name = "memoir", version, about = "Memoir recovery tool")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Decrypts a downloaded copy of the Memoir folder from Google Drive into
    /// Markdown files (folders, notes, replies, attachments, trash).
    Decrypt {
        /// The Memoir folder (with vault.json, objects and blobs), or the
        /// folder you extracted the Google Drive download into.
        folder: PathBuf,
        /// Where to write the Markdown files (a new or empty folder).
        #[arg(long)]
        out: PathBuf,
        /// Ask for the recovery key instead of the passphrase.
        #[arg(long)]
        recovery_key: bool,
    },
}

/// Prints warnings (e.g. an unreadable file) to stderr.
struct StderrLog;

impl log::Log for StderrLog {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        metadata.level() <= log::Level::Warn
    }
    fn log(&self, record: &log::Record<'_>) {
        if self.enabled(record.metadata()) {
            eprintln!("warning: {}", record.args());
        }
    }
    fn flush(&self) {}
}

fn main() -> ExitCode {
    let _ = log::set_logger(&StderrLog).map(|()| log::set_max_level(log::LevelFilter::Warn));
    let Cli { command } = Cli::parse();
    match command {
        Command::Decrypt {
            folder,
            out,
            recovery_key,
        } => {
            // Check what can be checked before asking for the secret.
            if let Err(e) = decrypt::find_vault_folder(&folder) {
                eprintln!("error: {e}");
                return ExitCode::FAILURE;
            }
            if std::fs::read_dir(&out).is_ok_and(|mut entries| entries.next().is_some()) {
                eprintln!("error: {} is not empty; choose a new folder", out.display());
                return ExitCode::FAILURE;
            }
            let prompt = if recovery_key {
                "Recovery key: "
            } else {
                "Passphrase: "
            };
            let secret = match rpassword::prompt_password(prompt) {
                Ok(s) => Zeroizing::new(s),
                Err(e) => {
                    eprintln!("error: can't read from the terminal: {e}");
                    return ExitCode::FAILURE;
                }
            };
            let secret = if recovery_key {
                Secret::RecoveryKey(secret)
            } else {
                Secret::Passphrase(secret)
            };
            let runtime = match tokio::runtime::Builder::new_current_thread().build() {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("error: {e}");
                    return ExitCode::FAILURE;
                }
            };
            eprintln!("Decrypting…");
            match runtime.block_on(decrypt(&folder, secret, &out)) {
                Ok(done) => {
                    let s = done.summary;
                    println!(
                        "Done: {} notes in {} folders, {} attachments (from {} revision files) → {}",
                        s.notes,
                        s.folders,
                        s.attachments,
                        done.revisions,
                        out.display()
                    );
                    if s.missing_attachments > 0 {
                        println!(
                            "{} attachments were missing from the folder; their links were left as they were.",
                            s.missing_attachments
                        );
                    }
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("error: {e}");
                    ExitCode::FAILURE
                }
            }
        }
    }
}
