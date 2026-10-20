//! Import an existing git repository into OpenScion.
//!
//! [`import_first_parent`] walks a git repository's first-parent history — the
//! mainline a sequence of merges produces — and writes it into an OpenScion
//! [`Repository`](openscion_core::Repository) as one changeset per spine
//! commit, reproducing each commit's tree and preserving the git commit id,
//! identities, and timestamps in changeset metadata. Git is read with
//! [`gix`](https://docs.rs/gix) (see `docs/adr/0005-git-ingestion-reader.md`);
//! the behavior is specified in `docs/specs/openscion-git-ingestion.md`.
//!
//! This crate reads git objects and drives the repository's public API
//! (`put_blob`, `create_changeset`, `update_ref`); it does not touch the stores
//! directly, nor does it reuse git's own object ids as OpenScion ids. It
//! imports only the first-parent spine: a merge's own tree is reproduced, but
//! its side branches are not walked. Submodule gitlinks have no OpenScion
//! equivalent and are skipped and counted.
//!
//! # Example
//!
//! ```no_run
//! use std::path::Path;
//! use openscion_core::Repository;
//! use openscion_git::import_first_parent;
//!
//! let mut repo = Repository::new();
//! let imported = import_first_parent(Path::new("/path/to/git/repo"), &mut repo, "HEAD", "main")?;
//! println!("imported {} changesets", imported.changesets);
//! # Ok::<(), openscion_git::Error>(())
//! ```

mod convert;
mod error;
mod import;

pub use error::{Error, Result};
pub use import::{Imported, import_first_parent};
