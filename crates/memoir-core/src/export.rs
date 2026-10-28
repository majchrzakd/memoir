//! Export to plain Markdown files: the notebook as a folder tree anyone can
//! open without Memoir. Used by the app ("Export to Markdown") and by the
//! recovery tool (`memoir decrypt`).
//!
//! Layout of the output folder:
//! - a directory per folder, a `.md` file per note (front matter with dates,
//!   tags and Later state; the note's replies appended at the end);
//! - `attachments/` with every attached file, linked from the notes by
//!   relative paths;
//! - links between notes (`note:<id>`) as relative links to the `.md` files;
//! - `Trash/` with whatever was in the trash.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

use crate::blobs::{BlobError, BlobStore, is_valid_id};
use crate::store::{Kind, NOTE_LINK_SCHEME, ObjectId, Store, StoreError};
use crate::sync::ObjectPayload;

pub const ATTACHMENTS_DIR: &str = "attachments";
pub const TRASH_DIR: &str = "Trash";

#[derive(Debug, thiserror::Error)]
pub enum ExportError {
    #[error("{0} is not empty; export into a new folder")]
    NotEmpty(PathBuf),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Blob(#[from] BlobError),
    #[error(transparent)]
    Io(#[from] io::Error),
}

pub type Result<T> = std::result::Result<T, ExportError>;

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct ExportSummary {
    pub folders: usize,
    pub notes: usize,
    pub attachments: usize,
    /// Attachments notes refer to that aren't on this device (not
    /// downloaded yet, or lost); their links are left as they were.
    pub missing_attachments: usize,
}

/// Writes the whole notebook (including the trash) to `out`, which must not
/// exist yet or be empty.
pub fn export_markdown(store: &Store, blobs: &BlobStore, out: &Path) -> Result<ExportSummary> {
    if out.exists() && fs::read_dir(out)?.next().is_some() {
        return Err(ExportError::NotEmpty(out.to_owned()));
    }
    fs::create_dir_all(out)?;

    let mut objects: BTreeMap<ObjectId, ObjectPayload> = BTreeMap::new();
    for id in store.all_object_ids()? {
        if let Some(p) = store.payload(&id)? {
            objects.insert(id, p);
        }
    }
    let notebook = Notebook::new(objects);
    let mut writer = Writer {
        notebook: &notebook,
        blobs,
        out,
        attachment_names: HashMap::new(),
        used_attachment_names: HashSet::new(),
        note_paths: HashMap::new(),
        planning: true,
        summary: ExportSummary::default(),
    };
    let top: Vec<&ObjectPayload> = notebook.children(None);
    let (live, trashed): (Vec<_>, Vec<_>) = top.into_iter().partition(|p| p.trashed_at.is_none());
    // Trashed items anywhere in the tree go to Trash/ (with their contents).
    let mut in_trash: Vec<&ObjectPayload> = trashed;
    in_trash.extend(notebook.trashed_below_live());
    in_trash.sort_by(|a, b| (&a.title, a.id).cmp(&(&b.title, b.id)));
    let trash = out.join(TRASH_DIR);

    // First every note's file name (for links between notes), then the files.
    for planning in [true, false] {
        writer.planning = planning;
        writer.write_items(&live, out, 0)?;
        if !in_trash.is_empty() {
            if !planning {
                fs::create_dir_all(&trash)?;
            }
            writer.write_items(&in_trash, &trash, 1)?;
        }
    }
    Ok(writer.summary)
}

/// The objects, indexed by parent.
struct Notebook {
    objects: BTreeMap<ObjectId, ObjectPayload>,
    children: HashMap<Option<ObjectId>, Vec<ObjectId>>,
}

impl Notebook {
    fn new(objects: BTreeMap<ObjectId, ObjectPayload>) -> Self {
        let mut children: HashMap<Option<ObjectId>, Vec<ObjectId>> = HashMap::new();
        for p in objects.values() {
            // A parent that isn't here (deleted elsewhere): top level.
            let parent = p.parent_id.filter(|id| objects.contains_key(id));
            children.entry(parent).or_default().push(p.id);
        }
        for list in children.values_mut() {
            list.sort_by(|a, b| {
                let (a, b) = (&objects[a], &objects[b]);
                (&a.sort_key, a.id).cmp(&(&b.sort_key, b.id))
            });
        }
        Self { objects, children }
    }

    fn is_reply(&self, p: &ObjectPayload) -> bool {
        p.kind == Kind::Note
            && p.parent_id
                .and_then(|id| self.objects.get(&id))
                .is_some_and(|parent| parent.kind == Kind::Note)
    }

    /// Folders and notes directly in `parent` (not replies).
    fn children(&self, parent: Option<ObjectId>) -> Vec<&ObjectPayload> {
        self.children
            .get(&parent)
            .into_iter()
            .flatten()
            .map(|id| &self.objects[id])
            .filter(|p| !self.is_reply(p))
            .collect()
    }

    /// A note's replies, oldest first.
    fn replies(&self, note: &ObjectId) -> Vec<&ObjectPayload> {
        let mut replies: Vec<&ObjectPayload> = self
            .children
            .get(&Some(*note))
            .into_iter()
            .flatten()
            .map(|id| &self.objects[id])
            .filter(|p| p.kind == Kind::Note)
            .collect();
        replies.sort_by_key(|r| (r.created_at, r.id));
        replies
    }

    /// Trashed items inside folders that aren't trashed themselves.
    fn trashed_below_live(&self) -> Vec<&ObjectPayload> {
        let mut found = Vec::new();
        let mut stack: Vec<&ObjectPayload> = self
            .children(None)
            .into_iter()
            .filter(|p| p.trashed_at.is_none() && p.kind == Kind::Folder)
            .collect();
        while let Some(folder) = stack.pop() {
            for child in self.children(Some(folder.id)) {
                if child.trashed_at.is_some() {
                    found.push(child);
                } else if child.kind == Kind::Folder {
                    stack.push(child);
                }
            }
        }
        found
    }
}

struct Writer<'a> {
    notebook: &'a Notebook,
    blobs: &'a BlobStore,
    out: &'a Path,
    /// Blob id → file name in `attachments/` (None: missing).
    attachment_names: HashMap<String, Option<String>>,
    used_attachment_names: HashSet<String>,
    /// Note id → its file, relative to `out`.
    note_paths: HashMap<ObjectId, PathBuf>,
    /// Only choosing file names (`note_paths`), nothing is written.
    planning: bool,
    summary: ExportSummary,
}

impl Writer<'_> {
    /// Writes `items` into `dir`, which is `depth` levels below the export
    /// root. Trashed items are skipped here unless `dir` is the trash.
    fn write_items(&mut self, items: &[&ObjectPayload], dir: &Path, depth: usize) -> Result<()> {
        let in_trash = dir.starts_with(self.out.join(TRASH_DIR));
        let mut names = UniqueNames::default();
        if depth == 0 {
            // Reserved at the top level.
            names.reserve(ATTACHMENTS_DIR);
            names.reserve(TRASH_DIR);
        }
        for item in items {
            if item.trashed_at.is_some() && !in_trash {
                continue; // written under Trash/
            }
            match item.kind {
                Kind::Folder => {
                    let name = names.take(&file_name(&item.title, "Untitled folder"));
                    let path = dir.join(&name);
                    if !self.planning {
                        fs::create_dir_all(&path)?;
                        self.summary.folders += 1;
                    }
                    let children = self.notebook.children(Some(item.id));
                    self.write_items(&children, &path, depth + 1)?;
                }
                Kind::Note => {
                    let name = names.take(&format!("{}.md", file_name(&item.title, "Untitled")));
                    let path = dir.join(&name);
                    if self.planning {
                        let relative = path.strip_prefix(self.out).unwrap_or(&path).to_owned();
                        self.note_paths.insert(item.id, relative);
                        continue;
                    }
                    let text = self.note_text(item, depth)?;
                    write_file(&path, text.as_bytes(), item.updated_at)?;
                    self.summary.notes += 1;
                }
            }
        }
        Ok(())
    }

    fn note_text(&mut self, note: &ObjectPayload, depth: usize) -> Result<String> {
        let mut text = String::from("---\n");
        text.push_str(&format!("title: {}\n", yaml_string(&note.title)));
        text.push_str(&format!("created: {}\n", iso_time(note.created_at)));
        text.push_str(&format!("updated: {}\n", iso_time(note.updated_at)));
        if !note.tags.is_empty() {
            let tags: Vec<String> = note.tags.iter().map(|t| yaml_string(t)).collect();
            text.push_str(&format!("tags: [{}]\n", tags.join(", ")));
        }
        if note.later_at.is_some() {
            text.push_str("later: true\n");
            if let Some(due) = &note.due_date {
                text.push_str(&format!("due: {due}\n"));
            }
        }
        if let Some(at) = note.trashed_at {
            text.push_str(&format!("trashed: {}\n", iso_time(at)));
        }
        text.push_str("---\n\n");
        text.push_str(&self.with_links(&note.body, depth)?);

        let replies = self.notebook.replies(&note.id);
        if !replies.is_empty() {
            ensure_blank_line(&mut text);
            text.push_str("---\n\n## Replies\n");
            for reply in replies {
                text.push_str(&format!("\n**{}**\n\n", human_time(reply.created_at)));
                text.push_str(&self.with_links(&reply.body, depth)?);
                ensure_blank_line(&mut text);
            }
        }
        if !text.ends_with('\n') {
            text.push('\n');
        }
        Ok(text)
    }

    /// The body with `attachment:<id>` links pointing at the exported files.
    fn with_links(&mut self, body: &str, depth: usize) -> Result<String> {
        const SCHEME: &str = "attachment:";
        let up = "../".repeat(depth);
        let mut out = String::with_capacity(body.len());
        let mut rest = body;
        while let Some(i) = rest.find(SCHEME) {
            let id: String = rest[i + SCHEME.len()..].chars().take(64).collect();
            out.push_str(&rest[..i]);
            if !is_valid_id(&id) {
                out.push_str(SCHEME);
                rest = &rest[i + SCHEME.len()..];
                continue;
            }
            let label = link_label(&out);
            match self.attachment(&id, label.as_deref())? {
                Some(name) => {
                    out.push_str(&up);
                    out.push_str(ATTACHMENTS_DIR);
                    out.push('/');
                    out.push_str(&percent_encode(&name));
                }
                None => {
                    out.push_str(SCHEME);
                    out.push_str(&id);
                }
            }
            rest = &rest[i + SCHEME.len() + id.len()..];
        }
        out.push_str(rest);
        Ok(self.with_note_links(&out, depth))
    }

    /// `note:<id>` links pointing at the exported notes' files. Links to
    /// notes that are gone are left as they were.
    fn with_note_links(&self, body: &str, depth: usize) -> String {
        let up = "../".repeat(depth);
        let mut out = String::with_capacity(body.len());
        let mut rest = body;
        while let Some(i) = rest.find(NOTE_LINK_SCHEME) {
            out.push_str(&rest[..i]);
            let after = &rest[i + NOTE_LINK_SCHEME.len()..];
            let id = after.get(..26).and_then(|id| id.parse::<ObjectId>().ok());
            match id.and_then(|id| self.note_paths.get(&id)) {
                Some(path) if out.ends_with("](") => {
                    out.push_str(&up);
                    let parts: Vec<String> = path
                        .iter()
                        .map(|part| percent_encode(&part.to_string_lossy()))
                        .collect();
                    out.push_str(&parts.join("/"));
                    rest = &after[26..];
                }
                _ => {
                    out.push_str(NOTE_LINK_SCHEME);
                    rest = after;
                }
            }
        }
        out.push_str(rest);
        out
    }

    /// Writes an attachment (once) and returns its file name, or None if
    /// it isn't on this device.
    fn attachment(&mut self, id: &str, label: Option<&str>) -> Result<Option<String>> {
        if let Some(known) = self.attachment_names.get(id) {
            return Ok(known.clone());
        }
        let data = match self.blobs.read(id) {
            Ok(data) => Some(data),
            Err(BlobError::NotFound(_) | BlobError::Crypto(_)) => None,
            Err(e) => return Err(e.into()),
        };
        let name = match data {
            None => {
                self.summary.missing_attachments += 1;
                None
            }
            Some(data) => {
                let mime = self.mime_of(id);
                let name = self.unique_attachment_name(id, label, mime.as_deref());
                let dir = self.out.join(ATTACHMENTS_DIR);
                fs::create_dir_all(&dir)?;
                write_file(&dir.join(&name), &data, 0)?;
                self.summary.attachments += 1;
                Some(name)
            }
        };
        self.attachment_names.insert(id.to_owned(), name.clone());
        Ok(name)
    }

    fn mime_of(&self, id: &str) -> Option<String> {
        self.notebook
            .objects
            .values()
            .flat_map(|p| &p.attachments)
            .find(|a| a.id == id)
            .map(|a| a.mime.clone())
    }

    fn unique_attachment_name(
        &mut self,
        id: &str,
        label: Option<&str>,
        mime: Option<&str>,
    ) -> String {
        let fallback = format!("attachment-{}", &id[..12]);
        let mut name = file_name(label.unwrap_or(""), &fallback);
        if Path::new(&name).extension().is_none()
            && let Some(ext) = mime.and_then(extension_for)
        {
            name = format!("{name}.{ext}");
        }
        let (stem, ext) = match name.rsplit_once('.') {
            Some((s, e)) if !s.is_empty() => (s.to_owned(), format!(".{e}")),
            _ => (name.clone(), String::new()),
        };
        let mut candidate = name;
        let mut n = 2;
        while !self.used_attachment_names.insert(candidate.to_lowercase()) {
            candidate = format!("{stem} ({n}){ext}");
            n += 1;
        }
        candidate
    }
}

/// File names unique within one directory (ignoring case, as Windows does).
#[derive(Default)]
struct UniqueNames(HashSet<String>);

impl UniqueNames {
    fn reserve(&mut self, name: &str) {
        self.0.insert(name.to_lowercase());
    }

    fn take(&mut self, name: &str) -> String {
        let (stem, ext) = match name.strip_suffix(".md") {
            Some(stem) => (stem, ".md"),
            None => (name, ""),
        };
        let mut candidate = name.to_owned();
        let mut n = 2;
        while !self.0.insert(candidate.to_lowercase()) {
            candidate = format!("{stem} ({n}){ext}");
            n += 1;
        }
        candidate
    }
}

/// A title made safe as a file name on Windows, macOS and Linux.
fn file_name(title: &str, fallback: &str) -> String {
    let cleaned: String = title
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    let mut name: String = cleaned.trim().chars().take(100).collect();
    while name.ends_with('.') || name.ends_with(' ') {
        name.pop();
    }
    if name.is_empty() {
        return fallback.to_owned();
    }
    let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.as_bytes()[3].is_ascii_digit());
    if reserved {
        name.insert(0, '_');
    }
    name
}

/// The text of the Markdown link whose destination starts right after
/// `before` (`[text](` or `![text](`), unescaped.
fn link_label(before: &str) -> Option<String> {
    let before = before.strip_suffix("](")?;
    // The last `[` that isn't escaped with a backslash.
    let start = before.rmatch_indices('[').map(|(i, _)| i).find(|&i| {
        let slashes = before[..i]
            .bytes()
            .rev()
            .take_while(|&b| b == b'\\')
            .count();
        slashes % 2 == 0
    })?;
    let raw = &before[start + 1..];
    let mut label = String::with_capacity(raw.len());
    let mut chars = raw.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(next) = chars.next() {
                label.push(next);
            }
        } else {
            label.push(c);
        }
    }
    let label = label.trim().to_owned();
    (!label.is_empty()).then_some(label)
}

fn extension_for(mime: &str) -> Option<&'static str> {
    Some(match mime {
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/svg+xml" => "svg",
        "image/bmp" => "bmp",
        "application/pdf" => "pdf",
        "text/plain" => "txt",
        "application/zip" => "zip",
        _ => return None,
    })
}

/// Encodes what a Markdown link destination can't hold as is.
fn percent_encode(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for b in name.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn yaml_string(s: &str) -> String {
    serde_json::to_string(s).expect("strings serialize") // JSON strings are valid YAML
}

fn ensure_blank_line(text: &mut String) {
    while !text.ends_with("\n\n") {
        text.push('\n');
    }
}

/// Writes a file and, if `modified_ms` > 0, sets its modification time.
fn write_file(path: &Path, data: &[u8], modified_ms: i64) -> Result<()> {
    let mut f = File::create(path)?;
    f.write_all(data)?;
    if modified_ms > 0 {
        f.set_modified(UNIX_EPOCH + Duration::from_millis(modified_ms as u64))?;
    }
    Ok(())
}

/// `2026-10-12T14:05:00Z`
fn iso_time(ms: i64) -> String {
    let (y, mo, d, h, mi, s) = civil(ms);
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}Z")
}

/// `2026-10-12 14:05 UTC`
fn human_time(ms: i64) -> String {
    let (y, mo, d, h, mi, _) = civil(ms);
    format!("{y:04}-{mo:02}-{d:02} {h:02}:{mi:02} UTC")
}

/// Unix ms → UTC date and time (proleptic Gregorian calendar).
fn civil(ms: i64) -> (i64, u32, u32, u32, u32, u32) {
    let secs = ms.div_euclid(1000);
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400) as u32;
    // Howard Hinnant's days-to-civil algorithm.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d, rem / 3600, rem / 60 % 60, rem % 60)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::{Keyring, SecretKey};
    use crate::store::Placement;

    fn setup() -> (tempfile::TempDir, Store, BlobStore) {
        let dir = tempfile::tempdir().unwrap();
        let keys = Keyring::derive(&SecretKey::from_bytes([4; 32]), 1);
        let store = Store::open(&dir.path().join("memoir.db"), &keys.local_db).unwrap();
        let blobs = BlobStore::open(dir.path().join("blobs"), keys).unwrap();
        (dir, store, blobs)
    }

    fn read(path: &Path) -> String {
        fs::read_to_string(path).unwrap()
    }

    #[test]
    fn exports_folders_notes_replies_attachments_and_trash() {
        let (dir, mut s, blobs) = setup();
        let work = s.create_folder(Placement::last_in(None), "Work").unwrap();
        let img = blobs.put(b"png bytes").unwrap();
        s.record_blob(&img, 9, "image/png").unwrap();
        let note = s
            .create_note(
                Placement::last_in(Some(work)),
                "Plan: Q4?",
                &format!("Hello\n\n![my chart](attachment:{img})\n"),
            )
            .unwrap();
        s.set_tags(&note, &["work".into()]).unwrap();
        s.add_reply(&note, "First reply\n").unwrap();
        let other = s
            .create_note(Placement::last_in(Some(work)), "Plan_ Q4_", "same name")
            .unwrap();
        let gone = s
            .create_note(Placement::last_in(None), "Old", "bye")
            .unwrap();
        s.move_to_trash(&gone).unwrap();
        let linking = s
            .create_note(
                Placement::last_in(None),
                "Linking",
                &format!(
                    "[plan](note:{note}) [old](note:{gone}) [gone](note:00000000000000000000000000)"
                ),
            )
            .unwrap();
        s.save_note(&gone, "Old", &format!("bye [back](note:{linking})"))
            .unwrap();
        let missing = "a".repeat(64);
        s.create_note(
            Placement::last_in(None),
            "",
            &format!("[file](attachment:{missing})"),
        )
        .unwrap();

        let out = dir.path().join("export");
        let summary = export_markdown(&s, &blobs, &out).unwrap();
        assert_eq!(
            summary,
            ExportSummary {
                folders: 1,
                notes: 5,
                attachments: 1,
                missing_attachments: 1
            }
        );

        let text = read(&out.join("Work").join("Plan_ Q4_.md"));
        assert!(text.starts_with("---\ntitle: \"Plan: Q4?\"\ncreated: "));
        assert!(text.contains("tags: [\"work\"]\n"));
        assert!(text.contains("![my chart](../attachments/my%20chart.png)"));
        assert!(text.contains("## Replies\n\n**"));
        assert!(text.contains("First reply"));
        assert_eq!(
            fs::read(out.join("attachments").join("my chart.png")).unwrap(),
            b"png bytes"
        );
        assert!(read(&out.join("Work").join("Plan_ Q4_ (2).md")).contains("same name"));
        assert!(read(&out.join("Trash").join("Old.md")).contains("trashed: "));
        assert!(read(&out.join("Untitled.md")).contains(&format!("attachment:{missing}")));
        let _ = other;
        // Links between notes point at the files, also from and into the trash.
        let linking = read(&out.join("Linking.md"));
        assert!(linking.contains("[plan](Work/Plan_%20Q4_.md)"), "{linking}");
        assert!(linking.contains("[old](Trash/Old.md)"));
        assert!(linking.contains("[gone](note:00000000000000000000000000)"));
        assert!(read(&out.join("Trash").join("Old.md")).contains("[back](../Linking.md)"));

        // Only into an empty folder.
        assert!(matches!(
            export_markdown(&s, &blobs, &out),
            Err(ExportError::NotEmpty(_))
        ));
    }

    #[test]
    fn file_names_are_safe() {
        assert_eq!(file_name("a/b\\c:d*?\"<>|", "x"), "a_b_c_d______");
        assert_eq!(file_name("  ...  ", "Untitled"), "Untitled");
        assert_eq!(file_name("Notes.", "x"), "Notes");
        assert_eq!(file_name("con", "x"), "_con");
        assert_eq!(file_name("COM1.txt", "x"), "_COM1.txt");
        assert_eq!(file_name("Comet", "x"), "Comet");
        assert_eq!(file_name(&"é".repeat(150), "x").chars().count(), 100);
    }

    #[test]
    fn link_labels_and_times() {
        assert_eq!(link_label("see ![a \\[b\\]](").as_deref(), Some("a [b]"));
        assert_eq!(link_label("plain text"), None);
        assert_eq!(iso_time(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso_time(1_792_958_800_000), "2026-10-25T20:06:40Z");
        assert_eq!(human_time(951_782_400_000), "2000-02-29 00:00 UTC");
    }
}
