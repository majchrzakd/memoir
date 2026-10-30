//! What a revision file contains, and how two versions of an object merge.

use serde::{Deserialize, Serialize};

use crate::store::{FolderLock, Kind, ObjectId};

/// When and where a field last changed. Ordered by time, then device id, so
/// every device picks the same winner.
#[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Clock {
    pub at: i64,
    #[serde(rename = "d")]
    pub device: String,
}

impl Clock {
    fn is_unset(&self) -> bool {
        *self == Self::default()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Clocks {
    pub title: Clock,
    pub body: Clock,
    /// Parent and position.
    pub parent: Clock,
    pub tags: Clock,
    pub trashed: Clock,
    pub later: Clock,
    /// Left out while unset, so objects without a password are written as
    /// before.
    #[serde(skip_serializing_if = "Clock::is_unset")]
    pub lock: Clock,
    pub deleted: Clock,
}

impl Clocks {
    /// The latest change to anything but deletion.
    fn content(&self) -> &Clock {
        [
            &self.title,
            &self.body,
            &self.parent,
            &self.tags,
            &self.trashed,
            &self.later,
            &self.lock,
        ]
        .into_iter()
        .max()
        .expect("non-empty")
    }

    /// Whether any field changed after the version `seen` describes.
    fn changed_since(&self, seen: &Clocks) -> bool {
        self.title > seen.title
            || self.body > seen.body
            || self.parent > seen.parent
            || self.tags > seen.tags
            || self.trashed > seen.trashed
            || self.later > seen.later
            || self.lock > seen.lock
    }

    /// Each field's clock becomes the later of `a`'s and `b`'s.
    fn seen_all(&mut self, a: &Clocks, b: &Clocks) {
        let later = |x: &Clock, y: &Clock| x.max(y).clone();
        self.title = later(&a.title, &b.title);
        self.body = later(&a.body, &b.body);
        self.parent = later(&a.parent, &b.parent);
        self.tags = later(&a.tags, &b.tags);
        self.trashed = later(&a.trashed, &b.trashed);
        self.later = later(&a.later, &b.later);
        self.lock = later(&a.lock, &b.lock);
        self.deleted = later(&a.deleted, &b.deleted);
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttachmentRef {
    pub id: String,
    pub size: u64,
    pub mime: String,
}

/// The full state of a folder, note or reply at one revision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectPayload {
    pub id: ObjectId,
    pub kind: Kind,
    /// A tombstone: the object was deleted for good.
    #[serde(default)]
    pub deleted: bool,
    pub parent_id: Option<ObjectId>,
    pub sort_key: String,
    pub title: String,
    /// Markdown (empty for folders).
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub tags: Vec<String>,
    pub created_at: i64,
    pub updated_at: i64,
    pub trashed_at: Option<i64>,
    pub later_at: Option<i64>,
    pub due_date: Option<String>,
    /// Attachments the body refers to, so other devices can fetch them.
    #[serde(default)]
    pub attachments: Vec<AttachmentRef>,
    /// A folder's password (its hash; see `FolderLock`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lock: Option<FolderLock>,
    #[serde(default)]
    pub clocks: Clocks,
}

/// Contents of a revision file (encrypted with the object key).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RevisionFile {
    pub v: u32,
    /// Revisions this one was based on (several after a merge).
    pub parents: Vec<String>,
    pub object: ObjectPayload,
}

pub const REVISION_FORMAT: u32 = 1;

/// Result of merging two versions of the same object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Merged {
    pub payload: ObjectPayload,
    /// When the text couldn't be merged: the losing version, as a new note
    /// next to the original, so nothing typed is lost.
    pub conflict_copy: Option<ObjectPayload>,
}

/// Merges `a` and `b`, two versions of one object that both descend from
/// `base` (None when the common version is unknown). Symmetric: swapping
/// `a` and `b` gives the same result, so every device merges alike.
pub fn merge(base: Option<&ObjectPayload>, a: &ObjectPayload, b: &ObjectPayload) -> Merged {
    let base = base.filter(|p| !p.deleted);
    let none = |payload| Merged {
        payload,
        conflict_copy: None,
    };

    if a == b {
        return none(a.clone());
    }
    // Deletion against edits. Changes the deleting device never saw (made
    // since the common version, offline elsewhere or after the deletion)
    // bring the object back: typed text is never dropped by a delete from a
    // device that didn't see it. Without a known common version, the
    // tombstone's clocks (those of the version the deleting device last
    // saw) stand in for it.
    match (a.deleted, b.deleted) {
        (true, true) => {
            let mut out = if a.clocks.deleted >= b.clocks.deleted {
                a
            } else {
                b
            }
            .clone();
            // The combined deletion has seen what either one saw.
            out.clocks.seen_all(&a.clocks, &b.clocks);
            return none(out);
        }
        (true, false) | (false, true) => {
            let (dead, alive) = if a.deleted { (a, b) } else { (b, a) };
            let unseen = match base {
                // Content, not clocks: a merge can bring in an older edit
                // without moving any clock forward.
                Some(base) => differs(alive, base),
                None => alive.clocks.changed_since(&dead.clocks),
            } || *alive.clocks.content() > dead.clocks.deleted;
            if !unseen {
                return none(dead.clone());
            }
            let mut revived = alive.clone();
            revived.clocks.deleted = dead.clocks.deleted.clone();
            return none(revived);
        }
        (false, false) => {}
    }

    // Field by field: the most recent change wins.
    let mut out = a.clone();
    if b.clocks.title > a.clocks.title {
        out.title.clone_from(&b.title);
        out.clocks.title = b.clocks.title.clone();
    }
    if b.clocks.parent > a.clocks.parent {
        out.parent_id = b.parent_id;
        out.sort_key.clone_from(&b.sort_key);
        out.clocks.parent = b.clocks.parent.clone();
    }
    if b.clocks.tags > a.clocks.tags {
        out.tags.clone_from(&b.tags);
        out.clocks.tags = b.clocks.tags.clone();
    }
    if b.clocks.trashed > a.clocks.trashed {
        out.trashed_at = b.trashed_at;
        out.clocks.trashed = b.clocks.trashed.clone();
    }
    if b.clocks.later > a.clocks.later {
        out.later_at = b.later_at;
        out.due_date.clone_from(&b.due_date);
        out.clocks.later = b.clocks.later.clone();
    }
    if b.clocks.lock > a.clocks.lock {
        out.lock.clone_from(&b.lock);
        out.clocks.lock = b.clocks.lock.clone();
    }
    out.clocks.deleted = a.clocks.deleted.clone().max(b.clocks.deleted.clone());
    out.created_at = a.created_at.min(b.created_at);
    out.updated_at = a.updated_at.max(b.updated_at);

    // Text: a 3-way merge; if the same lines changed on both sides, the more
    // recent version stays and the other becomes a conflict copy.
    let (older, newer) = if (&a.clocks.body, &a.body) <= (&b.clocks.body, &b.body) {
        (a, b)
    } else {
        (b, a)
    };
    let base_body = base.map_or("", |p| p.body.as_str());
    let mut conflict_copy = None;
    out.body = if older.body == newer.body || older.body == base_body {
        newer.body.clone()
    } else if newer.body == base_body {
        older.body.clone()
    } else {
        match diffy::merge(base_body, &older.body, &newer.body) {
            Ok(merged) => merged,
            Err(_) => {
                conflict_copy = Some(conflict_copy_of(older, &out));
                newer.body.clone()
            }
        }
    };
    out.clocks.body = newer.clocks.body.clone();
    out.attachments = attachments_in(&out.body, [a, b]);
    Merged {
        payload: out,
        conflict_copy,
    }
}

/// Whether `a` and `b` differ in anything a user changes.
fn differs(a: &ObjectPayload, b: &ObjectPayload) -> bool {
    a.title != b.title
        || a.body != b.body
        || a.parent_id != b.parent_id
        || a.sort_key != b.sort_key
        || a.tags != b.tags
        || a.trashed_at != b.trashed_at
        || a.later_at != b.later_at
        || a.due_date != b.due_date
        || a.lock != b.lock
}

/// The losing text as its own note, next to the original. Its id depends
/// only on the original and the losing text, so two devices resolving the
/// same conflict create the same copy instead of two.
fn conflict_copy_of(loser: &ObjectPayload, merged: &ObjectPayload) -> ObjectPayload {
    let mut seed = loser.id.to_string().into_bytes();
    seed.extend_from_slice(loser.body.as_bytes());
    let clock = loser.clocks.body.clone();
    let title = if loser.title.trim().is_empty() {
        "Conflict copy".to_owned()
    } else {
        format!("{} (conflict copy)", loser.title)
    };
    ObjectPayload {
        id: ObjectId::derived(&seed),
        kind: Kind::Note,
        deleted: false,
        parent_id: merged.parent_id,
        sort_key: merged.sort_key.clone(),
        title,
        body: loser.body.clone(),
        tags: loser.tags.clone(),
        created_at: clock.at,
        updated_at: clock.at,
        trashed_at: None,
        later_at: None,
        due_date: None,
        attachments: loser.attachments.clone(),
        lock: None,
        clocks: Clocks {
            title: clock.clone(),
            body: clock.clone(),
            parent: clock.clone(),
            tags: clock.clone(),
            trashed: clock.clone(),
            later: clock,
            lock: Clock::default(),
            deleted: Clock::default(),
        },
    }
}

/// Attachment ids referenced in Markdown (`attachment:<64 hex>`).
pub fn attachment_ids(body: &str) -> Vec<String> {
    let mut ids = Vec::new();
    for (i, _) in body.match_indices("attachment:") {
        let rest = &body[i + "attachment:".len()..];
        let id: String = rest.chars().take(64).collect();
        if crate::blobs::is_valid_id(&id) && !ids.contains(&id) {
            ids.push(id);
        }
    }
    ids
}

/// The attachments `body` refers to, with metadata from the given versions.
fn attachments_in<'a>(
    body: &str,
    versions: impl IntoIterator<Item = &'a ObjectPayload> + Clone,
) -> Vec<AttachmentRef> {
    attachment_ids(body)
        .into_iter()
        .filter_map(|id| {
            versions
                .clone()
                .into_iter()
                .flat_map(|v| &v.attachments)
                .find(|r| r.id == id)
                .cloned()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clock(at: i64, d: &str) -> Clock {
        Clock {
            at,
            device: d.into(),
        }
    }

    fn note(body: &str) -> ObjectPayload {
        let c = clock(1, "base");
        ObjectPayload {
            id: "01HZZZZZZZZZZZZZZZZZZZZZZZ".parse().unwrap(),
            kind: Kind::Note,
            deleted: false,
            parent_id: None,
            sort_key: "a0".into(),
            title: "Title".into(),
            body: body.into(),
            tags: vec![],
            created_at: 1,
            updated_at: 1,
            trashed_at: None,
            later_at: None,
            due_date: None,
            attachments: vec![],
            lock: None,
            clocks: Clocks {
                title: c.clone(),
                body: c.clone(),
                parent: c.clone(),
                tags: c.clone(),
                trashed: c.clone(),
                later: c,
                lock: Clock::default(),
                deleted: Clock::default(),
            },
        }
    }

    /// Both argument orders must agree.
    fn merge_both(base: &ObjectPayload, a: &ObjectPayload, b: &ObjectPayload) -> Merged {
        let ab = merge(Some(base), a, b);
        let ba = merge(Some(base), b, a);
        assert_eq!(ab, ba, "merge must be symmetric");
        ab
    }

    #[test]
    fn independent_field_changes_combine() {
        let base = note("text");
        let mut a = base.clone();
        a.title = "Renamed".into();
        a.clocks.title = clock(5, "A");
        let mut b = base.clone();
        b.parent_id = Some(ObjectId::generate());
        b.sort_key = "b0".into();
        b.clocks.parent = clock(6, "B");
        b.tags = vec!["x".into()];
        b.clocks.tags = clock(6, "B");

        let m = merge_both(&base, &a, &b).payload;
        assert_eq!(m.title, "Renamed");
        assert_eq!((m.parent_id, m.sort_key.as_str()), (b.parent_id, "b0"));
        assert_eq!(m.tags, ["x"]);
    }

    #[test]
    fn same_field_latest_wins_with_device_tie_break() {
        let base = note("text");
        let mut a = base.clone();
        a.title = "From A".into();
        a.clocks.title = clock(7, "A");
        let mut b = base.clone();
        b.title = "From B".into();
        b.clocks.title = clock(7, "B"); // same time: device id decides
        assert_eq!(merge_both(&base, &a, &b).payload.title, "From B");
        b.clocks.title = clock(6, "B");
        assert_eq!(merge_both(&base, &a, &b).payload.title, "From A");
    }

    #[test]
    fn edits_to_different_lines_merge() {
        let base = note("one\ntwo\nthree\n");
        let mut a = base.clone();
        a.body = "ONE\ntwo\nthree\n".into();
        a.clocks.body = clock(5, "A");
        let mut b = base.clone();
        b.body = "one\ntwo\nthree\nfour\n".into();
        b.clocks.body = clock(6, "B");
        let m = merge_both(&base, &a, &b);
        assert_eq!(m.payload.body, "ONE\ntwo\nthree\nfour\n");
        assert_eq!(m.conflict_copy, None);
    }

    #[test]
    fn conflicting_edits_keep_newer_and_copy_older() {
        let base = note("shopping: milk\n");
        let mut a = base.clone();
        a.body = "shopping: milk, eggs\n".into();
        a.clocks.body = clock(5, "A");
        let mut b = base.clone();
        b.body = "shopping: milk, bread\n".into();
        b.clocks.body = clock(6, "B");

        let m = merge_both(&base, &a, &b);
        assert_eq!(m.payload.body, b.body);
        let copy = m.conflict_copy.unwrap();
        assert_eq!(copy.body, a.body);
        assert_eq!(copy.title, "Title (conflict copy)");
        assert_ne!(copy.id, base.id);
        // Same conflict elsewhere → same copy id.
        assert_eq!(
            merge(Some(&base), &a, &b).conflict_copy.unwrap().id,
            copy.id
        );
    }

    #[test]
    fn unknown_base_only_merges_trivially() {
        let a = note("same");
        let mut b = a.clone();
        b.title = "x".into();
        b.clocks.title = clock(9, "B");
        assert_eq!(merge(None, &a, &b).payload.body, "same");
        let mut c = a.clone();
        c.body = "different".into();
        c.clocks.body = clock(9, "C");
        assert!(merge(None, &a, &c).conflict_copy.is_some());
    }

    #[test]
    fn deletion_wins_over_what_it_saw_but_not_over_unseen_edits() {
        let base = note("text");
        let mut dead = base.clone();
        dead.deleted = true;
        dead.clocks.deleted = clock(10, "A");

        // The other side has nothing the deleting device didn't see.
        assert!(merge_both(&base, &dead, &base).payload.deleted);

        // Edited offline before the deletion, but never seen by it.
        let mut edited = base.clone();
        edited.body = "edited".into();
        edited.clocks.body = clock(5, "B");
        let m = merge_both(&base, &dead, &edited).payload;
        assert!(!m.deleted);
        assert_eq!(m.body, "edited");

        // Edited after the deletion.
        edited.clocks.body = clock(11, "B");
        assert!(!merge_both(&base, &dead, &edited).payload.deleted);

        // An unseen edit counts even when the deleting device saw a later
        // edit from a third device: (base) -> seen (clock 8) -> deleted, and
        // separately (base) -> unseen (clock 6).
        let mut seen = base.clone();
        seen.body = "text
seen"
            .into();
        seen.clocks.body = clock(8, "C");
        let mut dead_after_seen = seen.clone();
        dead_after_seen.deleted = true;
        dead_after_seen.clocks.deleted = clock(9, "A");
        let mut unseen = base.clone();
        unseen.body = "unseen
text"
            .into();
        unseen.clocks.body = clock(6, "B");
        assert!(!merge_both(&base, &dead_after_seen, &unseen).payload.deleted);

        // The surviving side is a merge that brought in an older edit
        // without moving the body clock past what the deleting device saw.
        let mut merged = seen.clone();
        merged.body = "unseen
text
seen"
            .into();
        assert!(!merge_both(&seen, &dead_after_seen, &merged).payload.deleted);

        // A deletion made after seeing the edit wins (the common version is
        // then the edited one).
        let mut dead_later = edited.clone();
        dead_later.deleted = true;
        dead_later.clocks.deleted = clock(12, "A");
        assert!(merge_both(&edited, &dead_later, &edited).payload.deleted);
        // ...also once combined with the earlier deletion.
        let both = merge(None, &dead, &dead_later).payload;
        assert!(merge_both(&edited, &both, &edited).payload.deleted);

        // Without a common version, the tombstone's clocks decide.
        assert!(merge(None, &dead_later, &edited).payload.deleted);
        assert!(!merge(None, &dead, &edited).payload.deleted);
    }

    #[test]
    fn finds_attachment_references() {
        let a = "a".repeat(64);
        let b = "b".repeat(64);
        let body = format!(
            "![x](attachment:{a}) and [f](attachment:{b}) again attachment:{a} attachment:nope"
        );
        assert_eq!(attachment_ids(&body), [a, b]);
    }
}
