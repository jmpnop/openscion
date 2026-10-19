//! Minimal end-to-end usage of the engine: import two files, change one, and
//! read the snapshot back.
//!
//! Examples are compiled and linted in CI, so they cannot drift from the API.
//! Run it with:
//!
//! ```sh
//! cargo run -p openscion-core --example basic
//! ```

use openscion_core::{
    Change, CreateChangeset, EntryKind, ObjectId, RepoPath, Repository, Result, UpdateRef,
};

fn main() -> Result<()> {
    let mut repo = Repository::new();

    let readme = repo.put_blob(b"# demo\n")?;
    let main_rs = repo.put_blob(b"fn main() {}\n")?;
    let import = repo.create_changeset(CreateChangeset::new(
        vec![],
        vec![
            upsert("README.md", readme)?,
            upsert("src/main.rs", main_rs)?,
        ],
        "example",
        "import",
    ))?;
    repo.update_ref("main", UpdateRef::new(None, import.id))?;

    let edited = repo.put_blob(b"fn main() { println!(\"hi\"); }\n")?;
    let edit = repo.create_changeset(CreateChangeset::new(
        vec![import.id],
        vec![upsert("src/main.rs", edited)?],
        "example",
        "say hi",
    ))?;
    let main = repo.update_ref("main", UpdateRef::new(Some(import.id), edit.id))?;
    println!("main -> {} (version {})", main.target, main.version);

    for entry in repo.tree(edit.root_tree)? {
        println!("{:?} {} {}", entry.kind, entry.id, entry.name);
    }

    // Failure modes are part of the public contract; show one too.
    if let Err(error) = repo.update_ref("main", UpdateRef::new(Some(import.id), edit.id)) {
        println!("expected failure: {error}");
    }
    Ok(())
}

fn upsert(path: &str, blob: ObjectId) -> Result<Change> {
    Ok(Change::Upsert {
        path: RepoPath::new(path)?,
        kind: EntryKind::File,
        blob,
    })
}
