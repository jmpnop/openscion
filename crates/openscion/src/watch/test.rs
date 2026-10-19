//! Unit tests for the watcher's pure core and its event translation.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use notify::event::{AccessKind, CreateKind, DataChange, Flag, ModifyKind, RenameMode};
use notify::{Event, EventKind};
use openscion_core::{EntryKind, ObjectId};

use super::{
    Relativized, WatchEvent, WatchSession, arm, relativize, run, translate, watch, watch_error,
};
use crate::commands::StatusCode;
use crate::test_support::scratch;
use crate::workcopy::{self, FileState, Sparse, State};

/// Builds a working copy on disk with the given files tracked and clean, with no
/// server — enough for the local watcher to open and scan.
fn working_copy(files: &[(&str, &[u8])]) -> PathBuf {
    let root = scratch("watch");
    let mut tracked = BTreeMap::new();
    for (path, body) in files {
        let full = root.join(path);
        if let Some(parent) = full.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&full, body).unwrap();
        let (size, mtime, ctime) = workcopy::stat(&root, path).unwrap();
        tracked.insert(
            (*path).to_owned(),
            FileState {
                kind: EntryKind::File,
                blob: workcopy::blob_id(body),
                size,
                mtime,
                ctime,
            },
        );
    }
    let mut state = State {
        server: "http://127.0.0.1:1".to_owned(),
        ref_name: "main".to_owned(),
        base: ObjectId::from_bytes([0; 32]),
        sparse: Sparse::Full,
        written_at: 0,
        files: tracked,
    };
    workcopy::save(&root, &mut state).unwrap();
    root
}

/// The session's current report as a sorted vector.
fn report(session: &mut WatchSession) -> Vec<(String, StatusCode)> {
    session
        .refresh()
        .unwrap()
        .iter()
        .map(|(path, code)| (path.clone(), *code))
        .collect()
}

#[test]
fn the_first_refresh_is_a_full_status() {
    let root = working_copy(&[("a.txt", b"a"), ("b.txt", b"b")]);
    // The copy is already dirty before the session opens.
    fs::write(root.join("b.txt"), b"bb").unwrap();
    fs::write(root.join("c.txt"), b"c").unwrap();

    let mut session = WatchSession::open(&root).unwrap();
    let got = report(&mut session);
    assert_eq!(
        got,
        vec![
            ("b.txt".to_owned(), StatusCode::Modified),
            ("c.txt".to_owned(), StatusCode::Added),
        ]
    );
    // Identical to a full one-shot status.
    assert_eq!(got, crate::commands::status(&root).unwrap());
}

#[test]
fn a_changed_burst_updates_only_the_named_paths() {
    let root = working_copy(&[("a.txt", b"a"), ("b.txt", b"b")]);
    let mut session = WatchSession::open(&root).unwrap();
    assert!(report(&mut session).is_empty());

    fs::write(root.join("a.txt"), b"aa").unwrap();
    fs::write(root.join("c.txt"), b"c").unwrap();
    session.apply(WatchEvent::Changed(vec![
        "a.txt".to_owned(),
        "c.txt".to_owned(),
    ]));
    assert_eq!(
        report(&mut session),
        vec![
            ("a.txt".to_owned(), StatusCode::Modified),
            ("c.txt".to_owned(), StatusCode::Added),
        ]
    );

    // Revert a.txt to the base bytes and remove the new file; both drop off.
    fs::write(root.join("a.txt"), b"a").unwrap();
    fs::remove_file(root.join("c.txt")).unwrap();
    session.apply(WatchEvent::Changed(vec![
        "a.txt".to_owned(),
        "c.txt".to_owned(),
    ]));
    assert!(report(&mut session).is_empty());
}

#[test]
fn a_tracked_file_that_disappears_is_a_deletion() {
    let root = working_copy(&[("a.txt", b"a")]);
    let mut session = WatchSession::open(&root).unwrap();
    report(&mut session);
    fs::remove_file(root.join("a.txt")).unwrap();
    session.apply(WatchEvent::Changed(vec!["a.txt".to_owned()]));
    assert_eq!(
        report(&mut session),
        vec![("a.txt".to_owned(), StatusCode::Deleted)]
    );
}

#[test]
fn a_touch_with_identical_bytes_is_no_change() {
    let root = working_copy(&[("a.txt", b"same")]);
    let mut session = WatchSession::open(&root).unwrap();
    report(&mut session);
    // Rewrite with identical bytes: an event fires, but the content is the same.
    fs::write(root.join("a.txt"), b"same").unwrap();
    session.apply(WatchEvent::Changed(vec!["a.txt".to_owned()]));
    assert!(report(&mut session).is_empty());
}

#[test]
fn a_rescan_recomputes_the_whole_tree() {
    let root = working_copy(&[("a.txt", b"a")]);
    let mut session = WatchSession::open(&root).unwrap();
    report(&mut session);
    // A file appears but is never named; only a rescan can find it.
    fs::write(root.join("new.txt"), b"n").unwrap();
    session.apply(WatchEvent::Rescan);
    let got = report(&mut session);
    assert_eq!(got, vec![("new.txt".to_owned(), StatusCode::Added)]);
    assert_eq!(got, crate::commands::status(&root).unwrap());
}

#[test]
fn run_on_a_clean_copy_renders_clean_then_stops_when_the_sender_drops() {
    let root = working_copy(&[("a.txt", b"a")]);
    let mut session = WatchSession::open(&root).unwrap();
    let (sender, receiver) = mpsc::channel();
    drop(sender);
    let mut out = Vec::new();
    run(&receiver, &mut session, &mut out).unwrap();
    assert_eq!(String::from_utf8(out).unwrap(), "clean\n\n");
}

#[test]
fn run_renders_the_initial_state_and_each_burst() {
    let root = working_copy(&[("a.txt", b"a")]);
    let mut session = WatchSession::open(&root).unwrap();
    // Make the copy dirty, then feed one burst and close the channel.
    fs::write(root.join("a.txt"), b"aa").unwrap();
    let (sender, receiver) = mpsc::channel();
    // Two events in flight: the second is drained into the same render as the
    // first, so one burst is one render, not two.
    sender
        .send(WatchEvent::Changed(vec!["a.txt".to_owned()]))
        .unwrap();
    sender
        .send(WatchEvent::Changed(vec!["a.txt".to_owned()]))
        .unwrap();
    drop(sender);
    let mut out = Vec::new();
    run(&receiver, &mut session, &mut out).unwrap();
    // The initial render already sees the edit; the burst re-confirms it once.
    assert_eq!(String::from_utf8(out).unwrap(), "M a.txt\n\nM a.txt\n\n");
}

/// A watch whose output pipe is broken exits on its first render rather than
/// blocking forever — the way `opnscn watch | head` ends when the reader goes
/// away.
#[test]
fn watch_stops_when_its_output_breaks() {
    struct BrokenPipe;
    impl Write for BrokenPipe {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::Error::from(io::ErrorKind::BrokenPipe))
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let root = working_copy(&[("a.txt", b"a")]);
    let error = watch(&root, &mut BrokenPipe).unwrap_err();
    assert!(matches!(error, crate::Error::Output(_)));
}

#[test]
fn relativize_places_a_path_under_the_root() {
    let root = Path::new("/repo");
    assert_eq!(
        relativize(root, Path::new("/repo/src/lib.rs")),
        Relativized::Inside("src/lib.rs".to_owned())
    );
    assert_eq!(relativize(root, Path::new("/repo")), Relativized::Ignored);
    assert_eq!(
        relativize(root, Path::new("/repo/.opnscn/state.json")),
        Relativized::Ignored
    );
    assert_eq!(
        relativize(root, Path::new("/elsewhere/x")),
        Relativized::Outside
    );
    // A non-normal component under the root (a `..`) is treated conservatively.
    assert_eq!(
        relativize(root, Path::new("/repo/../repo/x")),
        Relativized::Outside
    );
}

#[cfg(unix)]
#[test]
fn relativize_rejects_a_non_utf8_path() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;
    let root = Path::new("/repo");
    let mut raw = b"/repo/".to_vec();
    raw.push(0xff); // not valid UTF-8
    let path = PathBuf::from(OsStr::from_bytes(&raw));
    assert_eq!(relativize(root, &path), Relativized::Outside);
}

#[test]
fn translate_maps_native_events_to_watch_events() {
    let root = Path::new("/repo");

    // A create of a tracked file is a change naming it.
    let event =
        Event::new(EventKind::Create(CreateKind::File)).add_path(PathBuf::from("/repo/a.txt"));
    assert_eq!(
        translate(root, Ok(event)),
        Some(WatchEvent::Changed(vec!["a.txt".to_owned()]))
    );

    // A content modification is a change too.
    let event = Event::new(EventKind::Modify(ModifyKind::Data(DataChange::Content)))
        .add_path(PathBuf::from("/repo/a.txt"));
    assert_eq!(
        translate(root, Ok(event)),
        Some(WatchEvent::Changed(vec!["a.txt".to_owned()]))
    );

    // A `.opnscn` write changes nothing the working copy tracks.
    let event = Event::new(EventKind::Modify(ModifyKind::Data(DataChange::Content)))
        .add_path(PathBuf::from("/repo/.opnscn/state.json"));
    assert_eq!(translate(root, Ok(event)), None);

    // A read touches no content.
    let event =
        Event::new(EventKind::Access(AccessKind::Read)).add_path(PathBuf::from("/repo/a.txt"));
    assert_eq!(translate(root, Ok(event)), None);

    // A rename cannot be attributed to specific files: rescan.
    let event = Event::new(EventKind::Modify(ModifyKind::Name(RenameMode::Both)))
        .add_path(PathBuf::from("/repo/a.txt"));
    assert_eq!(translate(root, Ok(event)), Some(WatchEvent::Rescan));

    // A path outside the root: rescan to be safe.
    let event =
        Event::new(EventKind::Create(CreateKind::File)).add_path(PathBuf::from("/elsewhere/x"));
    assert_eq!(translate(root, Ok(event)), Some(WatchEvent::Rescan));

    // An overflow flag forces a rescan whatever the kind.
    let event = Event::new(EventKind::Create(CreateKind::File))
        .add_path(PathBuf::from("/repo/a.txt"))
        .set_flag(Flag::Rescan);
    assert_eq!(translate(root, Ok(event)), Some(WatchEvent::Rescan));

    // A transport error is a rescan.
    assert_eq!(
        translate(root, Err(notify::Error::generic("boom"))),
        Some(WatchEvent::Rescan)
    );
}

#[test]
fn arm_watches_the_tree_and_forwards_events() {
    let root = working_copy(&[("a.txt", b"a")]);
    let (watcher, receiver, _session) = arm(&root).unwrap();
    fs::write(root.join("a.txt"), b"aa").unwrap();
    // The watcher's callback translates the native event onto the channel.
    let event = receiver
        .recv_timeout(Duration::from_secs(30))
        .expect("a filesystem event");
    match event {
        WatchEvent::Changed(paths) => assert!(paths.iter().any(|path| path == "a.txt")),
        // Some backends report a change they cannot attribute; a rescan is a
        // correct response to that and keeps the watch honest.
        WatchEvent::Rescan => {}
    }
    drop(watcher);
}

#[test]
fn a_watch_setup_failure_carries_its_cause() {
    let error = watch_error(&notify::Error::generic("no watcher"));
    assert!(matches!(error, crate::Error::Watch(message) if message.contains("no watcher")));
}
