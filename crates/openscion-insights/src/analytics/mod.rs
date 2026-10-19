//! The analytics plane: ingest changeset records, then query across history.

mod types;

use duckdb::{Connection, params};
use openscion_proto::{Change, ChangesetRecord, EntryKind};

use crate::Result;
pub use types::{AuthorActivity, PathChurn};

/// The schema: one row per changeset and one row per file-level change. Both
/// are append-only, which is why a columnar store fits — history only grows.
const SCHEMA: &str = "\
CREATE TABLE changesets (
    id           TEXT PRIMARY KEY,
    lsn          BIGINT,
    author       TEXT,
    created_at   BIGINT,
    parent_count INTEGER
);
CREATE TABLE changes (
    changeset_id TEXT,
    path         TEXT,
    op           TEXT,
    kind         TEXT,
    blob         TEXT
);";

/// An analytics plane over a snapshot of repository history.
///
/// Build one with [`Insights::in_memory`], load history with
/// [`Insights::ingest`], then ask questions. It holds its own database and
/// never reads a live store, so it is a read model, not a source of truth.
#[derive(Debug)]
pub struct Insights {
    conn: Connection,
}

impl Insights {
    /// An empty analytics plane in an in-memory database, schema created.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Database`](crate::Error::Database) when the database
    /// cannot be opened or the schema cannot be created.
    pub fn in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch(SCHEMA)?;
        Ok(Self { conn })
    }

    /// Ingests changeset records, exported from the log, into the read model.
    /// Idempotent per id only if `records` are distinct; callers export each
    /// changeset once.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Database`](crate::Error::Database) when an insert fails.
    pub fn ingest(&mut self, records: &[ChangesetRecord]) -> Result<()> {
        let transaction = self.conn.transaction()?;
        for record in records {
            let id = record.info.id.to_string();
            transaction.execute(
                "INSERT INTO changesets VALUES (?, ?, ?, ?, ?)",
                params![
                    id,
                    i64::try_from(record.info.lsn).unwrap_or(i64::MAX),
                    record.changeset.author,
                    i64::try_from(record.changeset.created_at).unwrap_or(i64::MAX),
                    i32::try_from(record.changeset.parents.len()).unwrap_or(i32::MAX),
                ],
            )?;
            for change in &record.changeset.changes {
                let (op, kind, blob) = describe(change);
                transaction.execute(
                    "INSERT INTO changes VALUES (?, ?, ?, ?, ?)",
                    params![id, change.path().as_str(), op, kind, blob],
                )?;
            }
        }
        transaction.commit()?;
        Ok(())
    }

    /// The `limit` most-changed paths, most changes first, ties broken by path.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Database`](crate::Error::Database) when the query fails.
    pub fn hotspots(&self, limit: usize) -> Result<Vec<PathChurn>> {
        let mut statement = self.conn.prepare(
            "SELECT path, COUNT(*) AS n FROM changes \
             GROUP BY path ORDER BY n DESC, path ASC LIMIT ?",
        )?;
        let rows = statement.query_map(params![as_i64(limit)], |row| {
            Ok(PathChurn {
                path: row.get(0)?,
                changes: count(row.get(1)?),
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// Every author and how many changesets they wrote, most first, ties broken
    /// by author.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Database`](crate::Error::Database) when the query fails.
    pub fn author_activity(&self) -> Result<Vec<AuthorActivity>> {
        let mut statement = self.conn.prepare(
            "SELECT author, COUNT(*) AS n FROM changesets \
             GROUP BY author ORDER BY n DESC, author ASC",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(AuthorActivity {
                author: row.get(0)?,
                changesets: count(row.get(1)?),
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// The `limit` paths that most often changed in the same changeset as
    /// `path`, most first, ties broken by path. `path` itself is excluded.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Database`](crate::Error::Database) when the query fails.
    pub fn co_changes(&self, path: &str, limit: usize) -> Result<Vec<PathChurn>> {
        let mut statement = self.conn.prepare(
            "SELECT b.path, COUNT(*) AS n \
             FROM changes a JOIN changes b ON a.changeset_id = b.changeset_id \
             WHERE a.path = ? AND b.path <> ? \
             GROUP BY b.path ORDER BY n DESC, b.path ASC LIMIT ?",
        )?;
        let rows = statement.query_map(params![path, path, as_i64(limit)], |row| {
            Ok(PathChurn {
                path: row.get(0)?,
                changes: count(row.get(1)?),
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }
}

/// A change as `(op, kind, blob)` columns: an upsert carries its kind and blob,
/// a delete carries neither.
fn describe(change: &Change) -> (&'static str, Option<&'static str>, Option<String>) {
    match change {
        Change::Upsert { kind, blob, .. } => {
            ("upsert", Some(kind_name(*kind)), Some(blob.to_string()))
        }
        Change::Delete { .. } => ("delete", None, None),
    }
}

/// The stored name of an entry kind.
fn kind_name(kind: EntryKind) -> &'static str {
    match kind {
        EntryKind::File => "file",
        EntryKind::Executable => "executable",
        EntryKind::Symlink => "symlink",
        EntryKind::Tree => "tree",
    }
}

/// A count column, which is a non-negative `BIGINT`, as a `u64`.
fn count(value: i64) -> u64 {
    u64::try_from(value).unwrap_or(0)
}

/// A limit as the `BIGINT` DuckDB expects, saturating rather than wrapping.
fn as_i64(limit: usize) -> i64 {
    i64::try_from(limit).unwrap_or(i64::MAX)
}

#[cfg(test)]
mod test;
