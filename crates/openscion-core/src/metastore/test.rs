//! Unit tests for the metadata stores.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use openscion_proto::{Changeset, CreateChangeset, ObjectId};

use super::{MemoryMetadataStore, MetadataStore, NewChangeset};

fn id(byte: u8) -> ObjectId {
    ObjectId::from_bytes([byte; 32])
}

fn changeset(parents: Vec<ObjectId>) -> Changeset {
    Changeset::from_request(CreateChangeset::new(parents, vec![], "agent", "m"), 7)
}

fn append(store: &mut MemoryMetadataStore, byte: u8, parents: Vec<ObjectId>, generation: u64) {
    store
        .append(NewChangeset {
            id: id(byte),
            root_tree: id(100 + byte),
            changeset: changeset(parents),
            generation,
            skips: Vec::new(),
        })
        .unwrap();
}

#[test]
fn append_assigns_sequential_sequence_numbers() {
    let mut store = MemoryMetadataStore::new();
    assert_eq!(store.log_len().unwrap(), 0);
    append(&mut store, 1, vec![], 1);
    append(&mut store, 2, vec![id(1)], 2);
    assert_eq!(store.log_len().unwrap(), 2);
    assert_eq!(store.info(id(1)).unwrap().unwrap().lsn, 1);
    assert_eq!(store.info(id(2)).unwrap().unwrap().lsn, 2);
    assert_eq!(store.info(id(2)).unwrap().unwrap().root_tree, id(102));
}

#[test]
fn the_three_read_widths_agree() {
    let mut store = MemoryMetadataStore::new();
    append(&mut store, 1, vec![], 1);
    append(&mut store, 2, vec![id(1)], 2);

    let info = store.info(id(2)).unwrap().unwrap();
    let ancestry = store.ancestry(id(2)).unwrap().unwrap();
    let record = store.record(id(2)).unwrap().unwrap();
    assert_eq!(record.info, info);
    assert_eq!(record.generation, ancestry.generation);
    assert_eq!(ancestry.generation, 2);
    assert_eq!(ancestry.parents, vec![id(1)]);
    assert_eq!(record.changeset.parents, vec![id(1)]);
}

#[test]
fn an_unknown_id_reads_as_none() {
    let store = MemoryMetadataStore::new();
    assert!(store.info(id(9)).unwrap().is_none());
    assert!(store.ancestry(id(9)).unwrap().is_none());
    assert!(store.record(id(9)).unwrap().is_none());
    assert!(store.first_parent_skips(id(9)).unwrap().is_none());
}

#[test]
fn the_skip_list_round_trips() {
    let mut store = MemoryMetadataStore::new();
    store
        .append(NewChangeset {
            id: id(1),
            root_tree: id(101),
            changeset: changeset(vec![]),
            generation: 1,
            skips: Vec::new(),
        })
        .unwrap();
    store
        .append(NewChangeset {
            id: id(3),
            root_tree: id(103),
            changeset: changeset(vec![id(1)]),
            generation: 2,
            skips: vec![id(1)],
        })
        .unwrap();
    assert!(store.first_parent_skips(id(1)).unwrap().unwrap().is_empty());
    assert_eq!(
        store.first_parent_skips(id(3)).unwrap().unwrap(),
        vec![id(1)]
    );
    assert_eq!(store.record(id(3)).unwrap().unwrap().skips, vec![id(1)]);
}

#[test]
fn a_ref_is_created_then_moved_by_compare_and_swap() {
    let mut store = MemoryMetadataStore::new();
    assert!(store.get_ref("main").unwrap().is_none());

    let created = store.update_ref("main", None, id(1)).unwrap();
    assert_eq!((created.target, created.version), (id(1), 1));
    assert_eq!(store.get_ref("main").unwrap().unwrap(), created);

    let moved = store.update_ref("main", Some(id(1)), id(2)).unwrap();
    assert_eq!((moved.target, moved.version), (id(2), 2));
}

#[test]
fn a_swap_against_the_wrong_expected_value_conflicts() {
    let mut store = MemoryMetadataStore::new();
    store.update_ref("main", None, id(1)).unwrap();

    // Creating a ref that already exists conflicts.
    let error = store.update_ref("main", None, id(2)).unwrap_err();
    assert!(matches!(
        &error,
        crate::Error::RefConflict { name, current } if name == "main" && *current == Some(id(1))
    ));
    // The ref is unchanged.
    assert_eq!(store.get_ref("main").unwrap().unwrap().target, id(1));

    // Moving from the wrong current value conflicts too.
    assert!(matches!(
        store.update_ref("main", Some(id(9)), id(2)),
        Err(crate::Error::RefConflict { .. })
    ));
}

#[cfg(feature = "sqlite")]
mod sqlite {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::{changeset, id};
    use crate::metastore::{MetadataStore, NewChangeset, SqliteMetadataStore};

    /// A fresh, non-existent database path under the temp directory.
    fn scratch_db() -> PathBuf {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "openscion-meta-{}-{}.db",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_file(&path);
        path
    }

    fn new_cs(byte: u8, parents: Vec<crate::ObjectId>, generation: u64) -> NewChangeset {
        NewChangeset {
            id: id(byte),
            root_tree: id(100 + byte),
            changeset: changeset(parents),
            generation,
            skips: Vec::new(),
        }
    }

    #[test]
    fn it_appends_reads_the_three_widths_and_counts() {
        let mut store = SqliteMetadataStore::in_memory().unwrap();
        assert_eq!(store.log_len().unwrap(), 0);
        let root = store.append(new_cs(1, vec![], 1)).unwrap();
        let child = store.append(new_cs(2, vec![id(1)], 2)).unwrap();
        assert_eq!((root.lsn, child.lsn), (1, 2));
        assert_eq!(store.log_len().unwrap(), 2);

        let info = store.info(id(2)).unwrap().unwrap();
        let ancestry = store.ancestry(id(2)).unwrap().unwrap();
        let record = store.record(id(2)).unwrap().unwrap();
        assert_eq!(info, child);
        assert_eq!(info.root_tree, id(102));
        assert_eq!(record.info, info);
        assert_eq!((ancestry.generation, record.generation), (2, 2));
        assert_eq!(ancestry.parents, vec![id(1)]);
        assert_eq!(record.changeset.parents, vec![id(1)]);
        assert_eq!(record.changeset.author, "agent");

        assert!(store.info(id(9)).unwrap().is_none());
        assert!(store.ancestry(id(9)).unwrap().is_none());
        assert!(store.record(id(9)).unwrap().is_none());
    }

    #[test]
    fn the_skip_list_persists_and_reads_back() {
        let mut store = SqliteMetadataStore::in_memory().unwrap();
        store.append(new_cs(1, vec![], 1)).unwrap();
        store
            .append(NewChangeset {
                id: id(3),
                root_tree: id(103),
                changeset: changeset(vec![id(1)]),
                generation: 2,
                skips: vec![id(1)],
            })
            .unwrap();
        assert!(store.first_parent_skips(id(1)).unwrap().unwrap().is_empty());
        assert_eq!(
            store.first_parent_skips(id(3)).unwrap().unwrap(),
            vec![id(1)]
        );
        assert_eq!(store.record(id(3)).unwrap().unwrap().skips, vec![id(1)]);
        assert!(store.first_parent_skips(id(9)).unwrap().is_none());
    }

    #[test]
    fn a_database_written_before_skip_lists_gains_the_column() {
        let path = scratch_db();
        // Build a changesets table without the skips column, as the pre-skip
        // schema did, and insert a row; reopening through the store migrates it.
        {
            let raw = rusqlite::Connection::open(&path).unwrap();
            raw.execute_batch(
                "CREATE TABLE changesets (
                     lsn        INTEGER PRIMARY KEY AUTOINCREMENT,
                     id         TEXT NOT NULL UNIQUE,
                     root_tree  TEXT NOT NULL,
                     generation INTEGER NOT NULL,
                     parents    TEXT NOT NULL,
                     payload    TEXT NOT NULL
                 );",
            )
            .unwrap();
            raw.execute(
                "INSERT INTO changesets (id, root_tree, generation, parents, payload)
                 VALUES (?1, ?2, 1, '[]', ?3)",
                rusqlite::params![
                    id(1).to_string(),
                    id(101).to_string(),
                    serde_json::to_string(&changeset(vec![])).unwrap(),
                ],
            )
            .unwrap();
        }
        let store = SqliteMetadataStore::open(&path).unwrap();
        // The migrated row reads back with an empty skip list, not an error.
        assert!(store.first_parent_skips(id(1)).unwrap().unwrap().is_empty());
        assert_eq!(store.record(id(1)).unwrap().unwrap().generation, 1);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn its_refs_compare_and_swap() {
        let mut store = SqliteMetadataStore::in_memory().unwrap();
        assert!(store.get_ref("main").unwrap().is_none());

        let created = store.update_ref("main", None, id(1)).unwrap();
        assert_eq!((created.target, created.version), (id(1), 1));
        let moved = store.update_ref("main", Some(id(1)), id(2)).unwrap();
        assert_eq!((moved.target, moved.version), (id(2), 2));
        assert_eq!(store.get_ref("main").unwrap().unwrap().target, id(2));

        // Creating a ref that exists, and moving from the wrong value, conflict.
        assert!(matches!(
            store.update_ref("main", None, id(3)),
            Err(crate::Error::RefConflict { current, .. }) if current == Some(id(2))
        ));
        assert!(matches!(
            store.update_ref("main", Some(id(9)), id(3)),
            Err(crate::Error::RefConflict { .. })
        ));
        assert_eq!(store.get_ref("main").unwrap().unwrap().target, id(2));
    }

    #[test]
    fn the_log_and_refs_survive_a_reopen() {
        let path = scratch_db();
        {
            let mut store = SqliteMetadataStore::open(&path).unwrap();
            store.append(new_cs(1, vec![], 1)).unwrap();
            store.update_ref("main", None, id(1)).unwrap();
        }
        // A fresh store on the same file sees the committed log and ref.
        let store = SqliteMetadataStore::open(&path).unwrap();
        assert_eq!(store.log_len().unwrap(), 1);
        assert_eq!(store.record(id(1)).unwrap().unwrap().info.lsn, 1);
        assert_eq!(store.get_ref("main").unwrap().unwrap().target, id(1));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_duplicate_append_is_a_database_error() {
        let mut store = SqliteMetadataStore::in_memory().unwrap();
        store.append(new_cs(1, vec![], 1)).unwrap();
        // The unique id constraint rejects a second append of the same id.
        assert!(matches!(
            store.append(new_cs(1, vec![], 1)),
            Err(crate::Error::Database { .. })
        ));
    }

    #[test]
    fn corrupt_stored_rows_are_database_errors() {
        let path = scratch_db();
        let store = SqliteMetadataStore::open(&path).unwrap();
        // Write rows the engine never would: a non-hex id and bad JSON, by a
        // second raw connection, then read them back through the store.
        let raw = rusqlite::Connection::open(&path).unwrap();
        raw.execute(
            "INSERT INTO changesets (id, root_tree, generation, parents, payload)
             VALUES (?1, 'not-an-id', 1, '[]', 'not json')",
            [id(1).to_string()],
        )
        .unwrap();
        raw.execute(
            "INSERT INTO refs (name, target, version) VALUES ('bad', 'not-an-id', 1)",
            [],
        )
        .unwrap();

        // A malformed root tree id, payload, and ref target each surface.
        assert!(matches!(
            store.info(id(1)),
            Err(crate::Error::Database { .. })
        ));
        assert!(matches!(
            store.record(id(1)),
            Err(crate::Error::Database { .. })
        ));
        assert!(matches!(
            store.get_ref("bad"),
            Err(crate::Error::Database { .. })
        ));
        // A malformed parents column fails ancestry's JSON decode too.
        raw.execute(
            "UPDATE changesets SET parents = 'not json' WHERE id = ?1",
            [id(1).to_string()],
        )
        .unwrap();
        assert!(matches!(
            store.ancestry(id(1)),
            Err(crate::Error::Database { .. })
        ));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_repository_runs_on_the_sqlite_store_and_its_metadata_persists() {
        use crate::{
            Change, CreateChangeset, EntryKind, FixedClock, MemoryObjectStore, RepoPath,
            Repository, UpdateRef,
        };

        let path = scratch_db();
        let changeset_id = {
            let mut repo = Repository::with_stores(
                Box::new(MemoryObjectStore::new()),
                Box::new(SqliteMetadataStore::open(&path).unwrap()),
                Box::new(FixedClock(0)),
            );
            let blob = repo.put_blob(b"hi").unwrap();
            let info = repo
                .create_changeset(CreateChangeset::new(
                    vec![],
                    vec![Change::Upsert {
                        path: RepoPath::new("a.txt").unwrap(),
                        kind: EntryKind::File,
                        blob,
                    }],
                    "agent",
                    "init",
                ))
                .unwrap();
            repo.update_ref("main", UpdateRef::new(None, info.id))
                .unwrap();
            info.id
        };

        // A fresh engine on the same metadata database sees the log and ref.
        // (Objects were in-memory, so only the metadata survives — the full
        // on-disk pairing is exercised by the server's data-dir wiring.)
        let repo = Repository::with_stores(
            Box::new(MemoryObjectStore::new()),
            Box::new(SqliteMetadataStore::open(&path).unwrap()),
            Box::new(FixedClock(0)),
        );
        assert_eq!(repo.log_len().unwrap(), 1);
        assert_eq!(repo.get_ref("main").unwrap().target, changeset_id);
        assert_eq!(repo.changeset(changeset_id).unwrap().info.id, changeset_id);
        let _ = std::fs::remove_file(&path);
    }
}
