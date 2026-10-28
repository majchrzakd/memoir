//! Importing notes from other apps. For now: Evernote export files
//! (`.enex`, see [`enex`]), whose content is converted to Markdown
//! ([`enml`]). Each imported note keeps its title, dates, tags and
//! attachments; it is then synced like any other.

pub mod enex;
pub mod enml;

use std::collections::HashMap;

use md5::{Digest, Md5};

use crate::blobs::{BlobError, BlobStore};
use crate::store::{ObjectId, Placement, Store, StoreError};
use enex::{EnexNote, EnexResource};

#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    #[error("{0}")]
    Format(String),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Blob(#[from] BlobError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// What an import brought in (added up over notes and files).
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct ImportSummary {
    pub notes: usize,
    pub attachments: usize,
    /// Attachments over the size limit, left out (a marker in the note says so).
    pub skipped_attachments: usize,
    /// Encrypted sections that couldn't be imported (marked in the note).
    pub encrypted_sections: usize,
}

impl ImportSummary {
    pub fn add(&mut self, other: &ImportSummary) {
        self.notes += other.notes;
        self.attachments += other.attachments;
        self.skipped_attachments += other.skipped_attachments;
        self.encrypted_sections += other.encrypted_sections;
    }
}

/// Adds one Evernote note to `folder`: its attachments go to the attachment
/// store, its content becomes Markdown that refers to them.
pub fn import_enex_note(
    store: &mut Store,
    blobs: &BlobStore,
    folder: ObjectId,
    note: &EnexNote,
    now: i64,
) -> Result<ImportSummary, ImportError> {
    let mut summary = ImportSummary {
        notes: 1,
        ..Default::default()
    };
    // Evernote refers to attachments by the MD5 of their bytes.
    let mut media: HashMap<String, String> = HashMap::new();
    let mut order: Vec<String> = Vec::new();
    for resource in &note.resources {
        let hash = hex(&Md5::digest(&resource.data));
        if media.contains_key(&hash) {
            continue;
        }
        let markdown = match blobs.put(&resource.data) {
            Ok(id) => {
                store.record_blob(&id, resource.data.len() as u64, &mime_of(resource))?;
                summary.attachments += 1;
                attachment_markdown(resource, &id)
            }
            Err(BlobError::TooLarge { .. }) => {
                summary.skipped_attachments += 1;
                format!(
                    "\\[Attachment too large to import: {}\\]",
                    escape_label(&display_name(resource))
                )
            }
            Err(e) => return Err(e.into()),
        };
        media.insert(hash.clone(), markdown);
        order.push(hash);
    }

    let converted = enml::to_markdown(&note.content, &mut |hash| media.get(hash).cloned());
    summary.encrypted_sections = converted.encrypted;
    let mut body = converted.markdown;
    // Attachments the text doesn't show (Evernote allows that) go at the end.
    for hash in order.iter().filter(|h| !converted.used_media.contains(*h)) {
        if !body.is_empty() {
            body.push_str("\n\n");
        }
        body.push_str(&media[hash]);
    }

    let created = note.created.or(note.updated).unwrap_or(now);
    let updated = note.updated.unwrap_or(created);
    store.create_imported_note(
        Placement::last_in(Some(folder)),
        &note.title,
        &body,
        created,
        updated,
        &note.tags,
    )?;
    Ok(summary)
}

fn attachment_markdown(resource: &EnexResource, id: &str) -> String {
    let name = display_name(resource);
    if resource.mime.starts_with("image/") {
        // The editor writes image names as they are.
        format!("![{name}](attachment:{id})")
    } else {
        format!("[{}](attachment:{id})", escape_label(&name))
    }
}

fn display_name(resource: &EnexResource) -> String {
    match &resource.file_name {
        Some(name) => name.clone(),
        None if resource.mime.starts_with("image/") => "image".into(),
        None => "attachment".into(),
    }
}

/// A valid MIME type, or the generic one.
fn mime_of(resource: &EnexResource) -> String {
    let m = resource.mime.trim();
    let valid = m.split_once('/').is_some_and(|(a, b)| {
        !a.is_empty()
            && !b.is_empty()
            && m.len() <= 100
            && m.bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"/+-.".contains(&c))
    });
    if valid {
        m.to_owned()
    } else {
        "application/octet-stream".into()
    }
}

fn escape_label(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if matches!(c, '\\' | '[' | ']' | '*' | '_' | '`') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::{Keyring, SecretKey};

    #[test]
    fn imports_a_note_with_dates_tags_and_attachments() {
        let dir = tempfile::tempdir().unwrap();
        let keys = Keyring::derive(&SecretKey::from_bytes([6; 32]), 1);
        let mut store = Store::open(&dir.path().join("memoir.db"), &keys.local_db).unwrap();
        let blobs = BlobStore::open(dir.path().join("blobs"), keys).unwrap();
        let folder = store
            .create_folder(Placement::last_in(None), "Travel")
            .unwrap();

        let png = b"\x89PNG fake image".to_vec();
        let pdf = b"%PDF fake".to_vec();
        let png_hash = hex(&Md5::digest(&png));
        let note = EnexNote {
            title: "Rome".into(),
            created: Some(1_600_000_000_000),
            updated: Some(1_600_000_500_000),
            tags: vec!["trips".into(), " Trips ".into(), "italy".into()],
            content: format!(
                r#"<en-note><div>Day 1</div><div><en-media type="image/png" hash="{png_hash}"/></div></en-note>"#
            ),
            resources: vec![
                EnexResource {
                    data: png.clone(),
                    mime: "image/png".into(),
                    file_name: Some("colosseum [1].png".into()),
                },
                EnexResource {
                    data: pdf,
                    mime: "application/pdf".into(),
                    file_name: None,
                },
            ],
        };
        let summary = import_enex_note(&mut store, &blobs, folder, &note, 0).unwrap();
        assert_eq!(
            summary,
            ImportSummary {
                notes: 1,
                attachments: 2,
                ..Default::default()
            }
        );

        let item = store
            .tree()
            .unwrap()
            .into_iter()
            .find(|i| i.title == "Rome")
            .unwrap();
        assert_eq!(item.parent_id, Some(folder));
        let n = store.note(&item.id).unwrap();
        assert_eq!(
            (n.created_at, n.updated_at),
            (1_600_000_000_000, 1_600_000_500_000)
        );
        assert_eq!(n.tags, ["trips", "italy"]);
        let png_id = blobs.put(&png).unwrap();
        assert!(n.body.starts_with(&format!(
            "Day 1\n\n![colosseum [1].png](attachment:{png_id})\n\n"
        )));
        // The PDF isn't shown in the text, so it's added at the end.
        assert!(n.body.trim_end().ends_with(")") && n.body.contains("[attachment](attachment:"));
        assert_eq!(store.blob(&png_id).unwrap().unwrap().mime, "image/png");
        assert_eq!(
            store.pending_changes().unwrap(),
            2,
            "folder and note queued for sync"
        );
    }
}
