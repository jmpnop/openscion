//! Unit tests for the analytics plane.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use openscion_proto::{
    Change, Changeset, ChangesetInfo, ChangesetRecord, CreateChangeset, EntryKind, ObjectId,
    RepoPath,
};

use super::{AuthorActivity, Insights, PathChurn};

fn id(byte: u8) -> ObjectId {
    ObjectId::from_bytes([byte; 32])
}

fn up(path: &str, kind: EntryKind, blob: u8) -> Change {
    Change::Upsert {
        path: RepoPath::new(path).unwrap(),
        kind,
        blob: id(blob),
    }
}

fn del(path: &str) -> Change {
    Change::Delete {
        path: RepoPath::new(path).unwrap(),
    }
}

fn record(byte: u8, lsn: u64, author: &str, changes: Vec<Change>) -> ChangesetRecord {
    ChangesetRecord::new(
        ChangesetInfo::new(id(byte), lsn, id(200)),
        Changeset::from_request(
            CreateChangeset::new(vec![], changes, author, "message"),
            lsn,
        ),
    )
}

fn loaded() -> Insights {
    let mut insights = Insights::in_memory().unwrap();
    insights
        .ingest(&[
            record(
                1,
                1,
                "alice",
                vec![
                    up("a.txt", EntryKind::File, 10),
                    up("src/main.rs", EntryKind::Executable, 11),
                ],
            ),
            record(
                2,
                2,
                "bob",
                vec![up("a.txt", EntryKind::File, 12), del("old.txt")],
            ),
            record(
                3,
                3,
                "alice",
                vec![
                    up("a.txt", EntryKind::File, 13),
                    up("link", EntryKind::Symlink, 14),
                    up("gen", EntryKind::Tree, 15),
                ],
            ),
        ])
        .unwrap();
    insights
}

#[test]
fn hotspots_rank_paths_by_change_count() {
    let insights = loaded();
    let top = insights.hotspots(10).unwrap();
    assert_eq!(
        top[0],
        PathChurn {
            path: "a.txt".to_owned(),
            changes: 3
        }
    );
    // Every other path changed once, so a.txt leads.
    assert!(top.iter().skip(1).all(|row| row.changes == 1));
    // The delete was recorded too.
    assert!(top.iter().any(|row| row.path == "old.txt"));
    // The limit caps the result.
    assert_eq!(insights.hotspots(1).unwrap().len(), 1);
}

#[test]
fn author_activity_counts_changesets_per_author() {
    let insights = loaded();
    assert_eq!(
        insights.author_activity().unwrap(),
        vec![
            AuthorActivity {
                author: "alice".to_owned(),
                changesets: 2
            },
            AuthorActivity {
                author: "bob".to_owned(),
                changesets: 1
            },
        ]
    );
}

#[test]
fn co_changes_finds_paths_that_move_together() {
    let insights = loaded();
    let together = insights.co_changes("a.txt", 10).unwrap();
    let paths: Vec<&str> = together.iter().map(|row| row.path.as_str()).collect();
    // a.txt shared a changeset with each of these, and never with itself.
    assert!(paths.contains(&"src/main.rs"));
    assert!(paths.contains(&"old.txt"));
    assert!(paths.contains(&"link"));
    assert!(paths.contains(&"gen"));
    assert!(!paths.contains(&"a.txt"));
    assert_eq!(together.len(), 4);
    // A path nothing moved with has no co-changes.
    assert!(
        insights
            .co_changes("src/main.rs", 10)
            .unwrap()
            .iter()
            .all(|r| r.path != "src/main.rs")
    );
}

#[test]
fn an_empty_plane_answers_with_nothing() {
    let insights = Insights::in_memory().unwrap();
    assert!(insights.hotspots(10).unwrap().is_empty());
    assert!(insights.author_activity().unwrap().is_empty());
    assert!(insights.co_changes("anything", 10).unwrap().is_empty());
}
