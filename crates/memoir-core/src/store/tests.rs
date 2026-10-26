use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use super::*;
use crate::vault::{self, KdfParams};

struct Fixture {
    _dir: tempfile::TempDir,
    path: std::path::PathBuf,
    store: Store,
    clock: Arc<AtomicI64>,
}

const KEY: [u8; 32] = [5; 32];

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("memoir.db");
    let mut store = Store::open(&path, &SecretKey::from_bytes(KEY)).unwrap();
    let clock = Arc::new(AtomicI64::new(1_000_000));
    let c = clock.clone();
    // Each call advances 1 ms, so timestamps are distinct and ordered.
    store.set_clock(move || c.fetch_add(1, Ordering::SeqCst));
    Fixture {
        _dir: dir,
        path,
        store,
        clock,
    }
}

fn last(parent: Option<ObjectId>) -> Placement {
    Placement::last_in(parent)
}

fn titles(items: &[TreeItem], parent: Option<ObjectId>) -> Vec<String> {
    items
        .iter()
        .filter(|i| i.parent_id == parent)
        .map(|i| i.title.clone())
        .collect()
}

fn outbox(store: &Store) -> Vec<ObjectId> {
    let mut stmt = store
        .conn
        .prepare("SELECT object_id FROM outbox ORDER BY object_id")
        .unwrap();
    stmt.query_map([], |r| r.get(0))
        .unwrap()
        .map(|r| r.unwrap())
        .collect()
}

#[test]
fn tree_orders_folders_first_then_by_position() {
    let mut f = fixture();
    let s = &mut f.store;
    let b = s.create_note(last(None), "B", "").unwrap();
    s.create_folder(last(None), "Folder 2").unwrap();
    s.create_note(
        Placement {
            parent: None,
            position: Position::First,
        },
        "A",
        "",
    )
    .unwrap();
    s.create_note(
        Placement {
            parent: None,
            position: Position::After(b),
        },
        "C",
        "",
    )
    .unwrap();
    s.create_folder(
        Placement {
            parent: None,
            position: Position::First,
        },
        "Folder 1",
    )
    .unwrap();
    s.create_note(
        Placement {
            parent: None,
            position: Position::Before(b),
        },
        "A2",
        "",
    )
    .unwrap();

    assert_eq!(
        titles(&s.tree().unwrap(), None),
        ["Folder 1", "Folder 2", "A", "A2", "B", "C"]
    );
}

#[test]
fn nested_folders_and_moves() {
    let mut f = fixture();
    let s = &mut f.store;
    let work = s.create_folder(last(None), "Work").unwrap();
    let projects = s.create_folder(last(Some(work)), "Projects").unwrap();
    let note = s.create_note(last(None), "Plan", "").unwrap();

    s.move_item(&note, last(Some(projects))).unwrap();
    let tree = s.tree().unwrap();
    assert_eq!(titles(&tree, Some(projects)), ["Plan"]);
    assert_eq!(titles(&tree, None), ["Work"]);

    // Reorder within the same folder.
    let other = s.create_note(last(Some(projects)), "Other", "").unwrap();
    s.move_item(
        &other,
        Placement {
            parent: Some(projects),
            position: Position::First,
        },
    )
    .unwrap();
    assert_eq!(
        titles(&s.tree().unwrap(), Some(projects)),
        ["Other", "Plan"]
    );

    assert!(matches!(
        s.move_item(&work, last(Some(projects))),
        Err(StoreError::Cycle)
    ));
    assert!(matches!(
        s.move_item(&work, last(Some(work))),
        Err(StoreError::Cycle)
    ));
    assert!(matches!(
        s.move_item(&projects, last(Some(note))),
        Err(StoreError::NotAFolder(_))
    ));
    assert!(matches!(
        s.move_item(
            &note,
            Placement {
                parent: None,
                position: Position::After(other)
            }
        ),
        Err(StoreError::NotASibling(_))
    ));
    assert!(matches!(
        s.create_note(last(Some(ObjectId::generate())), "x", ""),
        Err(StoreError::NotFound(_))
    ));
}

#[test]
fn rename_validates_folder_names() {
    let mut f = fixture();
    let s = &mut f.store;
    let folder = s.create_folder(last(None), "  Trips  ").unwrap();
    assert_eq!(titles(&s.tree().unwrap(), None), ["Trips"]);
    assert!(matches!(
        s.rename(&folder, "   "),
        Err(StoreError::EmptyName)
    ));
    assert!(matches!(
        s.create_folder(last(None), ""),
        Err(StoreError::EmptyName)
    ));

    let note = s.create_note(last(None), "", "").unwrap();
    s.rename(&note, "Groceries").unwrap();
    assert_eq!(s.search("grocer", 10).unwrap()[0].id, note);
}

#[test]
fn saving_unchanged_content_is_a_no_op() {
    let mut f = fixture();
    let s = &mut f.store;
    let note = s.create_note(last(None), "T", "body").unwrap();
    let before = s.note(&note).unwrap();
    s.save_note(&note, "T", "body").unwrap();
    s.set_tags(&note, &[]).unwrap();
    s.rename(&note, "T").unwrap();
    assert_eq!(s.note(&note).unwrap().updated_at, before.updated_at);

    s.save_note(&note, "T", "new body").unwrap();
    let after = s.note(&note).unwrap();
    assert_eq!(after.body, "new body");
    assert!(after.updated_at > before.updated_at);
}

#[test]
fn every_change_is_queued_once_per_object() {
    let mut f = fixture();
    let s = &mut f.store;
    let a = s.create_note(last(None), "a", "").unwrap();
    let b = s.create_folder(last(None), "b").unwrap();
    s.save_note(&a, "a", "1").unwrap();
    s.save_note(&a, "a", "2").unwrap();
    s.move_item(&a, last(Some(b))).unwrap();
    let mut expected = vec![a, b];
    expected.sort();
    assert_eq!(outbox(s), expected);
    assert_eq!(s.pending_changes().unwrap(), 2);
}

#[test]
fn trash_hides_folder_contents_and_restore_brings_them_back() {
    let mut f = fixture();
    let s = &mut f.store;
    let folder = s.create_folder(last(None), "Old").unwrap();
    let sub = s.create_folder(last(Some(folder)), "Sub").unwrap();
    let note = s.create_note(last(Some(sub)), "Inside", "findme").unwrap();

    s.move_to_trash(&folder).unwrap();
    assert!(s.tree().unwrap().is_empty());
    assert!(s.search("findme", 10).unwrap().is_empty());
    let trash: Vec<_> = s
        .trashed_items()
        .unwrap()
        .into_iter()
        .map(|i| i.id)
        .collect();
    assert_eq!(trash, [folder]);
    assert!(matches!(
        s.create_note(last(Some(sub)), "x", ""),
        Err(StoreError::InTrash(_))
    ));

    s.restore(&folder).unwrap();
    assert_eq!(s.tree().unwrap().len(), 3);
    assert_eq!(s.search("findme", 10).unwrap()[0].id, note);
    assert!(matches!(s.restore(&folder), Err(StoreError::NotInTrash(_))));
}

#[test]
fn restoring_into_a_trashed_folder_goes_to_top_level() {
    let mut f = fixture();
    let s = &mut f.store;
    let folder = s.create_folder(last(None), "F").unwrap();
    let note = s.create_note(last(Some(folder)), "N", "").unwrap();
    s.move_to_trash(&note).unwrap();
    s.move_to_trash(&folder).unwrap();

    s.restore(&note).unwrap();
    let tree = s.tree().unwrap();
    assert_eq!(tree.len(), 1);
    assert_eq!(tree[0].id, note);
    assert_eq!(tree[0].parent_id, None);
}

#[test]
fn purge_deletes_subtrees_and_queues_deletions() {
    let mut f = fixture();
    let s = &mut f.store;
    let folder = s.create_folder(last(None), "F").unwrap();
    let note = s.create_note(last(Some(folder)), "N", "gone").unwrap();
    let keep = s.create_note(last(None), "Keep", "").unwrap();

    assert!(matches!(s.purge(&folder), Err(StoreError::NotInTrash(_))));
    s.move_to_trash(&folder).unwrap();
    s.purge(&folder).unwrap();

    assert!(matches!(s.note(&note), Err(StoreError::NotFound(_))));
    assert!(s.trashed_items().unwrap().is_empty());
    assert!(s.search("gone", 10).unwrap().is_empty());
    let fts_rows: i64 = s
        .conn
        .query_row("SELECT count(*) FROM notes_fts", [], |r| r.get(0))
        .unwrap();
    assert_eq!(fts_rows, 1);
    // Deleted ids stay in the outbox as tombstones.
    let mut expected = vec![folder, note, keep];
    expected.sort();
    assert_eq!(outbox(s), expected);
}

#[test]
fn purge_trashed_before_cutoff() {
    let mut f = fixture();
    let a = f.store.create_note(last(None), "a", "").unwrap();
    let b = f.store.create_note(last(None), "b", "").unwrap();
    f.store.move_to_trash(&a).unwrap();
    let cutoff = f.clock.load(Ordering::SeqCst);
    f.store.move_to_trash(&b).unwrap();

    assert_eq!(f.store.purge_trashed_before(cutoff).unwrap(), 1);
    assert_eq!(f.store.trashed_items().unwrap()[0].id, b);
    assert_eq!(f.store.empty_trash().unwrap(), 1);
    assert!(f.store.trashed_items().unwrap().is_empty());
}

#[test]
fn search_ranks_strips_markdown_and_marks_matches() {
    let mut f = fixture();
    let s = &mut f.store;
    let title_hit = s
        .create_note(last(None), "Kraków trip", "Plans for the weekend")
        .unwrap();
    let body_hit = s
        .create_note(
            last(None),
            "Misc",
            "We could visit [Kraków](https://krakow.example) one day",
        )
        .unwrap();
    let tagged = s.create_note(last(None), "Receipts", "").unwrap();
    s.set_tags(
        &tagged,
        &[
            "finance".into(),
            " Finance ".into(),
            "".into(),
            "tax".into(),
        ],
    )
    .unwrap();
    assert_eq!(s.note(&tagged).unwrap().tags, ["finance", "tax"]);

    let hits = s.search("krakow", 10).unwrap();
    assert_eq!(
        hits.iter().map(|h| h.id).collect::<Vec<_>>(),
        [title_hit, body_hit]
    );
    assert!(
        hits[1]
            .snippet
            .contains(&format!("{MATCH_START}Kraków{MATCH_END}"))
    );

    assert_eq!(s.search("trip plan", 10).unwrap()[0].id, title_hit);
    assert_eq!(s.search("fina", 10).unwrap()[0].id, tagged);
    assert!(
        s.search("example", 10).unwrap().is_empty(),
        "URLs are not indexed"
    );
    assert!(s.search("   ", 10).unwrap().is_empty());
    // FTS syntax in user input must not cause errors.
    for weird in ["\"", "a OR", "NEAR(", "*", "-x", "col:val", "(", "^"] {
        s.search(weird, 10).unwrap();
    }
}

fn bodies(store: &Store, id: &ObjectId) -> Vec<String> {
    store
        .history(id)
        .unwrap()
        .into_iter()
        .map(|v| v.body)
        .collect()
}

#[test]
fn history_keeps_the_version_before_a_burst_and_after_a_pause() {
    let mut f = fixture();
    const MIN: i64 = 60_000;
    let at = |f: &Fixture, t: i64| f.clock.store(t, Ordering::SeqCst);

    at(&f, 0);
    let note = f.store.create_note(last(None), "T", "v1").unwrap();
    // A burst of autosaves, one a minute: only the original is kept.
    for (i, body) in ["v2", "v3", "v4", "v5"].iter().enumerate() {
        at(&f, (i as i64 + 1) * MIN);
        f.store.save_note(&note, "T", body).unwrap();
    }
    assert_eq!(bodies(&f.store, &note), ["v1"]);

    // Editing again after a 20-minute pause keeps the settled v5.
    at(&f, 24 * MIN);
    f.store.save_note(&note, "T", "v6").unwrap();
    assert_eq!(bodies(&f.store, &note), ["v5", "v1"]);

    // Restoring keeps the version it replaces.
    let v1 = f.store.history(&note).unwrap()[1].saved_at;
    f.store.restore_version(&note, v1).unwrap();
    assert_eq!(f.store.note(&note).unwrap().body, "v1");
    assert_eq!(bodies(&f.store, &note), ["v6", "v5", "v1"]);
    assert!(matches!(
        f.store.restore_version(&note, 42),
        Err(StoreError::NoSuchVersion(..))
    ));
}

#[test]
fn history_checkpoints_long_continuous_editing() {
    let mut f = fixture();
    const MIN: i64 = 60_000;
    f.clock.store(0, Ordering::SeqCst);
    let note = f.store.create_note(last(None), "T", "v0").unwrap();
    // 35 minutes of saving once a minute.
    for i in 1..=35 {
        f.clock.store(i * MIN, Ordering::SeqCst);
        f.store.save_note(&note, "T", &format!("v{i}")).unwrap();
    }
    assert_eq!(bodies(&f.store, &note), ["v30", "v20", "v10", "v0"]);
}

#[test]
fn history_is_capped() {
    let mut f = fixture();
    let note = f.store.create_note(last(None), "T", "v0").unwrap();
    for i in 1..=(HISTORY_MAX_VERSIONS + 10) {
        f.clock.store(i * HISTORY_INTERVAL_MS * 2, Ordering::SeqCst);
        f.store.save_note(&note, "T", &format!("v{i}")).unwrap();
    }
    assert_eq!(
        f.store.history(&note).unwrap().len() as i64,
        HISTORY_MAX_VERSIONS
    );
}

#[test]
fn orphans_are_shown_at_top_level() {
    let mut f = fixture();
    let s = &mut f.store;
    let folder = s.create_folder(last(None), "F").unwrap();
    let note = s.create_note(last(Some(folder)), "N", "").unwrap();
    // As if another device purged the folder and sync removed only it.
    s.conn
        .execute("DELETE FROM objects WHERE id = ?1", [folder])
        .unwrap();

    let tree = s.tree().unwrap();
    assert_eq!(tree.len(), 1);
    assert_eq!((tree[0].id, tree[0].parent_id), (note, None));
}

#[test]
fn data_vault_and_device_id_survive_reopening() {
    let mut f = fixture();
    let new_vault = vault::create(
        "correct horse battery",
        KdfParams {
            m_kib: 64,
            t: 1,
            p: 1,
        },
    )
    .unwrap();
    f.store
        .save_vault(&new_vault.file, &new_vault.vault)
        .unwrap();
    let note = f
        .store
        .create_note(last(None), "Persisted", "body")
        .unwrap();
    let device = f.store.device_id().unwrap();
    drop(f.store);

    let store = Store::open(&f.path, &SecretKey::from_bytes(KEY)).unwrap();
    assert_eq!(store.note(&note).unwrap().title, "Persisted");
    assert_eq!(store.device_id().unwrap(), device);
    let (file, vault) = store.load_vault().unwrap().unwrap();
    assert_eq!(file, new_vault.file);
    assert_eq!(
        vault.keys().object.as_bytes(),
        new_vault.vault.keys().object.as_bytes()
    );

    drop(store);
    assert!(Store::open(&f.path, &SecretKey::from_bytes([6; 32])).is_err());
}

#[test]
fn rejects_database_from_newer_app() {
    let f = fixture();
    f.store
        .conn
        .pragma_update(None, "user_version", 99)
        .unwrap();
    drop(f.store);
    assert!(matches!(
        Store::open(&f.path, &SecretKey::from_bytes(KEY)),
        Err(StoreError::NewerSchema(99))
    ));
}

#[test]
fn later_list_is_sorted_by_due_date_then_creation() {
    let mut f = fixture();
    let s = &mut f.store;
    let a = s.create_note(last(None), "a", "").unwrap();
    let b = s.create_note(last(None), "b", "").unwrap();
    let c = s.create_note(last(None), "c", "").unwrap();
    let d = s.create_note(last(None), "d", "").unwrap();
    let e = s.create_note(last(None), "e", "").unwrap();
    s.create_note(last(None), "not later", "").unwrap();
    let folder = s.create_folder(last(None), "Old").unwrap();
    let hidden = s.create_note(last(Some(folder)), "in trash", "").unwrap();

    s.set_later(&d, true, None).unwrap();
    s.set_later(&c, true, Some("2026-11-05")).unwrap();
    s.set_later(&a, true, None).unwrap();
    s.set_later(&b, true, Some("2026-11-01")).unwrap();
    s.set_later(&e, true, Some("2026-11-01")).unwrap();
    s.set_later(&hidden, true, None).unwrap();
    s.move_to_trash(&folder).unwrap();

    let ids: Vec<ObjectId> = s.later_notes().unwrap().into_iter().map(|i| i.id).collect();
    // Dated (soonest first; same date: newest first), then undated, newest first.
    assert_eq!(ids, [e, b, c, d, a]);
}

#[test]
fn set_later_validates_and_clears() {
    let mut f = fixture();
    let s = &mut f.store;
    let note = s.create_note(last(None), "n", "").unwrap();

    s.set_later(&note, true, None).unwrap();
    let marked_at = s.note(&note).unwrap().later_at.unwrap();
    // Changing only the due date keeps when it was marked.
    s.set_later(&note, true, Some("2024-02-29")).unwrap();
    let n = s.note(&note).unwrap();
    assert_eq!(
        (n.later_at, n.due_date.as_deref()),
        (Some(marked_at), Some("2024-02-29"))
    );

    for bad in [
        "2026-02-29",
        "2026-13-01",
        "2026-1-05",
        "26-01-05",
        "tomorrow",
        "2026-04-31",
        "",
    ] {
        assert!(
            matches!(
                s.set_later(&note, true, Some(bad)),
                Err(StoreError::InvalidDate(_))
            ),
            "{bad}"
        );
    }

    s.set_later(&note, false, Some("2026-01-01")).unwrap();
    let n = s.note(&note).unwrap();
    assert_eq!((n.later_at, n.due_date), (None, None));
    assert!(s.later_notes().unwrap().is_empty());

    let folder = s.create_folder(last(None), "F").unwrap();
    assert!(matches!(
        s.set_later(&folder, true, None),
        Err(StoreError::NotANote(_))
    ));
}

#[test]
fn edited_time_reflects_content_only() {
    let mut f = fixture();
    let s = &mut f.store;
    let folder = s.create_folder(last(None), "F").unwrap();
    let note = s.create_note(last(None), "n", "").unwrap();
    let created = s.note(&note).unwrap().updated_at;

    s.move_item(&note, last(Some(folder))).unwrap();
    s.set_later(&note, true, Some("2026-12-24")).unwrap();
    s.move_to_trash(&note).unwrap();
    s.restore(&note).unwrap();
    assert_eq!(s.note(&note).unwrap().updated_at, created);
    // ...but every one of those changes is still queued for sync.
    assert!(outbox(s).contains(&note));

    s.save_note(&note, "n", "text").unwrap();
    assert!(s.note(&note).unwrap().updated_at > created);
}

#[test]
fn threads_hold_replies_outside_the_tree() {
    let mut f = fixture();
    let s = &mut f.store;
    let note = s.create_note(last(None), "Project", "main text").unwrap();
    let first = s.add_reply(&note, "first update").unwrap();
    let second = s.add_reply(&note, "second update").unwrap();

    let replies = s.replies(&note).unwrap();
    assert_eq!(
        replies.iter().map(|r| r.id).collect::<Vec<_>>(),
        [first, second]
    );
    assert_eq!(replies[0].body, "first update");

    // Replies are not tree items; the note carries the count.
    let tree = s.tree().unwrap();
    assert_eq!(tree.len(), 1);
    assert_eq!((tree[0].id, tree[0].replies), (note, 2));

    // Editing a reply uses save_note.
    s.save_note(&first, "", "first update, edited").unwrap();
    assert_eq!(s.replies(&note).unwrap()[0].body, "first update, edited");

    s.delete_reply(&first).unwrap();
    assert_eq!(s.tree().unwrap()[0].replies, 1);
    assert!(matches!(s.note(&first), Err(StoreError::NotFound(_))));
    assert!(outbox(s).contains(&first), "deletion is queued for sync");
}

#[test]
fn threads_are_one_level_and_replies_stay_with_their_note() {
    let mut f = fixture();
    let s = &mut f.store;
    let folder = s.create_folder(last(None), "F").unwrap();
    let note = s.create_note(last(None), "N", "").unwrap();
    let reply = s.add_reply(&note, "r").unwrap();

    assert!(matches!(
        s.add_reply(&reply, "x"),
        Err(StoreError::Reply(_))
    ));
    assert!(matches!(
        s.add_reply(&folder, "x"),
        Err(StoreError::NotANote(_))
    ));
    assert!(matches!(
        s.move_item(&reply, last(None)),
        Err(StoreError::Reply(_))
    ));
    assert!(matches!(s.move_to_trash(&reply), Err(StoreError::Reply(_))));
    assert!(matches!(
        s.set_later(&reply, true, None),
        Err(StoreError::Reply(_))
    ));
    assert!(matches!(
        s.delete_reply(&note),
        Err(StoreError::NotAReply(_))
    ));
    // Nothing can be placed inside a note except through add_reply.
    assert!(matches!(
        s.create_note(last(Some(note)), "x", ""),
        Err(StoreError::NotAFolder(_))
    ));

    s.move_to_trash(&note).unwrap();
    assert!(matches!(
        s.add_reply(&note, "x"),
        Err(StoreError::InTrash(_))
    ));
}

#[test]
fn search_finds_replies_and_points_at_their_note() {
    let mut f = fixture();
    let s = &mut f.store;
    let note = s
        .create_note(last(None), "Renovation", "kitchen plans")
        .unwrap();
    let reply = s.add_reply(&note, "Plumber comes on Tuesday").unwrap();

    let hits = s.search("plumber", 10).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(
        (hits[0].id, hits[0].reply_id, hits[0].title.as_str()),
        (note, Some(reply), "Renovation")
    );
    assert_eq!(s.search("kitchen", 10).unwrap()[0].reply_id, None);

    // Trashing the note hides its replies; purging deletes them.
    s.move_to_trash(&note).unwrap();
    assert!(s.search("plumber", 10).unwrap().is_empty());
    assert!(s.trashed_items().unwrap().iter().all(|i| i.id != reply));
    s.purge(&note).unwrap();
    assert!(matches!(s.note(&reply), Err(StoreError::NotFound(_))));
}

#[test]
fn attachment_metadata_is_recorded_once() {
    let mut f = fixture();
    let id = "ab".repeat(32);
    assert_eq!(f.store.blob(&id).unwrap(), None);
    f.store.record_blob(&id, 1234, "image/png").unwrap();
    f.store
        .record_blob(&id, 1234, "application/octet-stream")
        .unwrap();
    assert_eq!(
        f.store.blob(&id).unwrap(),
        Some(BlobMeta {
            id: id.clone(),
            size: 1234,
            mime: "image/png".into()
        })
    );
}

#[test]
fn remote_state_is_stored_and_cleared() {
    let mut f = fixture();
    assert_eq!(f.store.remote_value("provider").unwrap(), None);
    f.store
        .set_remote_values(&[("provider", "gdrive"), ("account", "me@example.com")])
        .unwrap();
    f.store
        .set_remote_values(&[("account", "other@example.com")])
        .unwrap();
    assert_eq!(
        f.store.remote_value("provider").unwrap().as_deref(),
        Some("gdrive")
    );
    assert_eq!(
        f.store.remote_value("account").unwrap().as_deref(),
        Some("other@example.com")
    );
    f.store.clear_remote_state().unwrap();
    assert_eq!(f.store.remote_value("provider").unwrap(), None);
}

#[test]
fn saving_from_an_editor_merges_changes_made_underneath() {
    let mut f = fixture();
    let s = &mut f.store;
    let id = s
        .create_note(last(None), "Plan", "one\ntwo\nthree\n")
        .unwrap();

    // Nothing changed underneath: a plain save.
    let r = s
        .save_note_from(
            &id,
            "Plan",
            "one\ntwo\nthree\n",
            "Plan",
            "one\ntwo\nthree\nfour\n",
        )
        .unwrap();
    assert!(r.is_none());

    // Sync renamed it and changed the first line while the editor (still
    // showing the old version) added a line at the end.
    s.save_note(&id, "Plan B", "ONE\ntwo\nthree\nfour\n")
        .unwrap();
    let merged = s
        .save_note_from(
            &id,
            "Plan",
            "one\ntwo\nthree\nfour\n",
            "Plan",
            "one\ntwo\nthree\nfour\nfive\n",
        )
        .unwrap()
        .expect("merged");
    assert_eq!(merged.title, "Plan B", "the editor didn't touch the title");
    assert_eq!(merged.body, "ONE\ntwo\nthree\nfour\nfive\n");
    assert_eq!(s.note(&id).unwrap().body, merged.body);
}

#[test]
fn saving_from_an_editor_keeps_both_versions_on_conflict() {
    let mut f = fixture();
    let s = &mut f.store;
    let folder = s.create_folder(last(None), "F").unwrap();
    let id = s
        .create_note(last(Some(folder)), "Plan", "same line\n")
        .unwrap();
    let reply = s.add_reply(&id, "reply\n").unwrap();

    s.save_note(&id, "Plan", "their line\n").unwrap();
    let kept = s
        .save_note_from(&id, "Plan", "same line\n", "Plan", "my line\n")
        .unwrap()
        .expect("changed");
    assert_eq!(kept.body, "my line\n");
    let tree = s.tree().unwrap();
    let copy = tree
        .iter()
        .find(|i| i.title == "Plan (conflict copy)")
        .expect("copy");
    assert_eq!(copy.parent_id, Some(folder));
    assert_eq!(s.note(&copy.id).unwrap().body, "their line\n");

    // A reply's conflict copy is another reply.
    s.save_note(&reply, "", "their reply\n").unwrap();
    s.save_note_from(&reply, "", "reply\n", "", "my reply\n")
        .unwrap();
    let bodies: Vec<String> = s
        .replies(&id)
        .unwrap()
        .into_iter()
        .map(|r| r.body)
        .collect();
    assert_eq!(bodies, ["my reply\n", "their reply\n"]);
}

#[test]
fn saves_record_only_the_fields_that_changed() {
    let mut f = fixture();
    let s = &mut f.store;
    let id = s.create_note(last(None), "Title", "body").unwrap();
    let before = s.payload(&id).unwrap().unwrap().clocks;
    s.save_note(&id, "Title", "new body").unwrap();
    let after = s.payload(&id).unwrap().unwrap().clocks;
    assert_eq!(after.title, before.title);
    assert!(after.body > before.body);
}

#[test]
fn never_synced_objects_are_queued() {
    let mut f = fixture();
    let s = &mut f.store;
    let a = s.create_note(last(None), "A", "").unwrap();
    s.conn.execute("DELETE FROM outbox", []).unwrap();
    assert_eq!(s.enqueue_never_synced().unwrap(), 1);
    assert_eq!(outbox(s), [a]);
    assert_eq!(s.enqueue_never_synced().unwrap(), 0);
}

#[test]
fn superseded_revisions_list_ancestors_first() {
    let mut f = fixture();
    let s = &mut f.store;
    let id = s.create_note(last(None), "N", "").unwrap();
    // Chain r3 -> r2 -> r1 (ids sort the other way round).
    for (rev, parents) in [("r3", vec![]), ("r2", vec!["r3"]), ("r1", vec!["r2"])] {
        s.record_remote_revision(&id, rev, &format!("file-{rev}"), 10)
            .unwrap();
        let parents: Vec<String> = parents.into_iter().map(str::to_owned).collect();
        s.set_revision_content(rev, &parents, None).unwrap();
    }
    let order: Vec<String> = s
        .superseded_revisions(i64::MAX)
        .unwrap()
        .into_iter()
        .map(|(rev, _)| rev)
        .collect();
    assert_eq!(order, ["r3", "r2"]);
}

#[test]
fn text_replaced_by_sync_is_kept_in_history() {
    let mut f = fixture();
    let id = f.store.create_note(last(None), "N", "mine").unwrap();
    // Later than the history interval: worth keeping.
    f.clock.fetch_add(HISTORY_INTERVAL_MS, Ordering::SeqCst);
    let mut theirs = f.store.payload(&id).unwrap().unwrap();
    theirs.body = "theirs".into();
    let sync = f.store.sync_object(&id).unwrap();
    f.store.apply_synced(&theirs, &sync, false, None).unwrap();
    assert_eq!(f.store.note(&id).unwrap().body, "theirs");
    assert_eq!(bodies(&f.store, &id), ["mine"]);
}
