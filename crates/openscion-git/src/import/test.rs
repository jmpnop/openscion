//! Integration tests: import real git repositories (built with the `git` CLI,
//! with a fixed identity and date so the provenance is deterministic) and
//! assert the OpenScion history they become.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use openscion_core::{EntryKind, ObjectId, Repository};

use super::import_first_parent;

/// A fixed commit date, so `git.*_time` metadata is deterministic.
const DATE: &str = "1700000000 +0000";

/// A throwaway git repository under the temp directory, driven by the `git`
/// CLI with an isolated config and a fixed identity and date.
struct GitRepo {
    dir: PathBuf,
}

impl GitRepo {
    fn init(name: &str) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "openscion-git-{}-{name}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let repo = Self { dir };
        repo.git(&["init", "-q", "-b", "main"]);
        repo
    }

    /// Runs `git` in the repo with an isolated config and fixed identity/date,
    /// returning its trimmed stdout.
    fn git(&self, args: &[&str]) -> String {
        self.run(args)
    }

    /// Like [`git`](Self::git), but accepts `OsStr` arguments so a path that is
    /// not UTF-8 can be passed to git plumbing.
    fn run<S: AsRef<OsStr> + std::fmt::Debug>(&self, args: &[S]) -> String {
        let output = Command::new("git")
            .args(args)
            .current_dir(&self.dir)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .env("GIT_AUTHOR_NAME", "Test User")
            .env("GIT_AUTHOR_EMAIL", "test@example.com")
            .env("GIT_COMMITTER_NAME", "Test User")
            .env("GIT_COMMITTER_EMAIL", "test@example.com")
            .env("GIT_AUTHOR_DATE", DATE)
            .env("GIT_COMMITTER_DATE", DATE)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    }

    fn write(&self, path: &str, bytes: &[u8]) {
        let full = self.dir.join(path);
        std::fs::create_dir_all(full.parent().unwrap()).unwrap();
        std::fs::write(full, bytes).unwrap();
    }

    fn commit_all(&self, message: &str) -> String {
        self.git(&["add", "-A"]);
        self.git(&["commit", "-q", "-m", message]);
        self.git(&["rev-parse", "HEAD"])
    }

    fn remove_and_commit(&self, path: &str, message: &str) -> String {
        std::fs::remove_file(self.dir.join(path)).unwrap();
        self.commit_all(message)
    }
}

/// The full snapshot at `tree`, as path → (kind, bytes), walking subtrees.
fn snapshot(
    repo: &Repository,
    tree: ObjectId,
    prefix: &str,
    out: &mut BTreeMap<String, (EntryKind, Vec<u8>)>,
) {
    for entry in repo.tree(tree).unwrap() {
        let path = format!("{prefix}{}", entry.name);
        if entry.kind == EntryKind::Tree {
            snapshot(repo, entry.id, &format!("{path}/"), out);
        } else {
            out.insert(path, (entry.kind, repo.object(entry.id).unwrap().to_vec()));
        }
    }
}

/// The snapshot at the changeset `ref_name` points at.
fn snapshot_of(repo: &Repository, ref_name: &str) -> BTreeMap<String, (EntryKind, Vec<u8>)> {
    let head = repo.get_ref(ref_name).unwrap().target;
    let root = repo.changeset(head).unwrap().info.root_tree;
    let mut out = BTreeMap::new();
    snapshot(repo, root, "", &mut out);
    out
}

/// The commit messages along the first-parent chain from the ref, newest first.
fn first_parent_messages(repo: &Repository, ref_name: &str) -> Vec<String> {
    let mut cursor = Some(repo.get_ref(ref_name).unwrap().target);
    let mut out = Vec::new();
    while let Some(id) = cursor {
        let record = repo.changeset(id).unwrap();
        out.push(record.changeset.message.clone());
        cursor = record.changeset.parents.first().copied();
    }
    out
}

#[test]
fn imports_linear_history_as_first_parent_changesets() {
    let git = GitRepo::init("linear");
    git.write("src/a.txt", b"one\n");
    let c1 = git.commit_all("first\n");
    git.write("src/a.txt", b"two\n");
    git.write("b.txt", b"bee\n");
    git.commit_all("second\n");
    git.remove_and_commit("b.txt", "third\n");

    let mut repo = Repository::new();
    let imported = import_first_parent(&git.dir, &mut repo, "HEAD", "main").unwrap();
    assert_eq!(imported.changesets, 3);
    assert_eq!(imported.skipped_submodules, 0);

    // First-parent order, newest first.
    assert_eq!(
        first_parent_messages(&repo, "main"),
        ["third\n", "second\n", "first\n"]
    );

    // The tip snapshot: a.txt edited, b.txt deleted in the third commit.
    let snap = snapshot_of(&repo, "main");
    assert_eq!(snap.len(), 1);
    assert_eq!(snap["src/a.txt"], (EntryKind::File, b"two\n".to_vec()));

    // Author and provenance metadata on the tip changeset.
    let head = repo.get_ref("main").unwrap().target;
    let record = repo.changeset(head).unwrap();
    assert_eq!(record.changeset.author, "Test User <test@example.com>");
    assert_eq!(
        record.changeset.metadata["git.commit"],
        git.git(&["rev-parse", "HEAD"])
    );
    assert_eq!(record.changeset.metadata["git.author_time"], DATE);

    // The root changeset carries the first git commit's id.
    let root_id = {
        let mut cursor = head;
        loop {
            let rec = repo.changeset(cursor).unwrap();
            match rec.changeset.parents.first() {
                Some(parent) => cursor = *parent,
                None => break cursor,
            }
        }
    };
    assert_eq!(
        repo.changeset(root_id).unwrap().changeset.metadata["git.commit"],
        c1
    );
}

#[cfg(unix)]
#[test]
fn reproduces_files_executables_and_symlinks() {
    use std::os::unix::fs::PermissionsExt;

    let git = GitRepo::init("kinds");
    git.write("run.sh", b"#!/bin/sh\necho hi\n");
    std::fs::set_permissions(
        git.dir.join("run.sh"),
        std::fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    git.write("plain.txt", b"plain\n");
    std::os::unix::fs::symlink("plain.txt", git.dir.join("link")).unwrap();
    git.commit_all("kinds\n");

    let mut repo = Repository::new();
    import_first_parent(&git.dir, &mut repo, "HEAD", "main").unwrap();

    let snap = snapshot_of(&repo, "main");
    assert_eq!(snap["run.sh"].0, EntryKind::Executable);
    assert_eq!(snap["plain.txt"], (EntryKind::File, b"plain\n".to_vec()));
    // A symlink is stored as its target path's bytes.
    assert_eq!(snap["link"], (EntryKind::Symlink, b"plain.txt".to_vec()));
}

#[test]
fn imports_only_the_first_parent_spine_of_a_merge() {
    let git = GitRepo::init("merge");
    git.write("base.txt", b"base\n");
    git.commit_all("base\n");
    // A side branch adds its own file.
    git.git(&["checkout", "-q", "-b", "side"]);
    git.write("side.txt", b"from side\n");
    git.commit_all("side\n");
    // Back on main, a mainline commit, then merge side in (no fast-forward).
    git.git(&["checkout", "-q", "main"]);
    git.write("main.txt", b"from main\n");
    git.commit_all("main\n");
    git.git(&["merge", "--no-ff", "-q", "-m", "merge\n", "side"]);

    let mut repo = Repository::new();
    let imported = import_first_parent(&git.dir, &mut repo, "HEAD", "main").unwrap();
    // Spine: base, main, merge — three, not the side commit.
    assert_eq!(imported.changesets, 3);
    assert_eq!(
        first_parent_messages(&repo, "main"),
        ["merge\n", "main\n", "base\n"]
    );
    // The merge's own tree still reproduces the merged-in side file.
    let snap = snapshot_of(&repo, "main");
    assert!(snap.contains_key("side.txt"));
    assert!(snap.contains_key("main.txt"));
    assert!(snap.contains_key("base.txt"));
}

#[test]
fn skips_and_counts_submodule_gitlinks() {
    let git = GitRepo::init("submodule");
    git.write("keep.txt", b"keep\n");
    git.git(&["add", "-A"]);
    // A gitlink entry (mode 160000) pointing at an arbitrary commit id.
    let fake = "0123456789abcdef0123456789abcdef01234567";
    git.git(&[
        "update-index",
        "--add",
        "--cacheinfo",
        &format!("160000,{fake},sub"),
    ]);
    git.git(&["commit", "-q", "-m", "with submodule\n"]);

    let mut repo = Repository::new();
    let imported = import_first_parent(&git.dir, &mut repo, "HEAD", "main").unwrap();
    assert_eq!(imported.skipped_submodules, 1);
    let snap = snapshot_of(&repo, "main");
    assert!(snap.contains_key("keep.txt"));
    assert!(!snap.contains_key("sub"));
}

#[cfg(unix)]
#[test]
fn a_non_utf8_path_is_an_error() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStrExt;

    // A non-UTF-8 name cannot be created on every filesystem (APFS rejects it),
    // so stage the entry through git's index with plumbing, where the path is
    // just bytes: hash a blob, then add a tree entry under a non-UTF-8 name.
    let git = GitRepo::init("non-utf8");
    git.write("content", b"x\n");
    let blob = git.git(&["hash-object", "-w", "content"]);
    let mut cacheinfo = format!("100644,{blob},").into_bytes();
    cacheinfo.extend_from_slice(b"bad-\xff-name");
    let args: Vec<OsString> = vec![
        "update-index".into(),
        "--add".into(),
        "--cacheinfo".into(),
        OsStr::from_bytes(&cacheinfo).to_os_string(),
    ];
    git.run(&args);
    git.git(&["commit", "-q", "-m", "bad name\n"]);

    let mut repo = Repository::new();
    assert!(matches!(
        import_first_parent(&git.dir, &mut repo, "HEAD", "main"),
        Err(crate::Error::NonUtf8Path(_))
    ));
}

#[test]
fn opening_a_non_repository_is_a_git_error() {
    let dir = std::env::temp_dir().join(format!("openscion-git-empty-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut repo = Repository::new();
    assert!(matches!(
        import_first_parent(&dir, &mut repo, "HEAD", "main"),
        Err(crate::Error::Git(_))
    ));
}
