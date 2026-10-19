//! The landing commands: `rebase`, `revert`, and `land`.
//!
//! Each resolves its revisions through [`resolve_rev`](super::resolve_rev),
//! asks the server to move the changes onto a new base, and prints the
//! resulting changeset. `land` names a ref to move rather than a base to sit
//! on, and prints the ref's new version too.

use std::io::Write;

use super::resolve_rev;
use crate::{Error, Result, ServerClient};

/// Rebases `rev` onto `onto` and prints the resulting changeset.
pub(crate) fn rebase(
    client: &ServerClient,
    rev: &str,
    onto: &str,
    out: &mut impl Write,
) -> Result<()> {
    let changeset = resolve_rev(client, rev)?;
    let onto = resolve_rev(client, onto)?;
    let result = client.rebase(changeset, onto)?;
    writeln!(out, "{}", result.changeset).map_err(Error::Output)
}

/// Reverts `rev` onto `onto` and prints the resulting changeset.
pub(crate) fn revert(
    client: &ServerClient,
    rev: &str,
    onto: &str,
    author: &str,
    message: &str,
    out: &mut impl Write,
) -> Result<()> {
    let changeset = resolve_rev(client, rev)?;
    let onto = resolve_rev(client, onto)?;
    let result = client.revert(changeset, onto, author, message)?;
    writeln!(out, "{}", result.changeset).map_err(Error::Output)
}

/// Lands `rev` on `ref_name` by compare-and-swap, retrying up to `attempts`
/// times, and prints the landed changeset and the ref's new version.
pub(crate) fn land(
    client: &ServerClient,
    rev: &str,
    ref_name: &str,
    attempts: u32,
    out: &mut impl Write,
) -> Result<()> {
    let changeset = resolve_rev(client, rev)?;
    let result = client.land(changeset, ref_name, attempts)?;
    writeln!(
        out,
        "landed {} on {ref_name} at version {}",
        result.changeset, result.version
    )
    .map_err(Error::Output)
}
