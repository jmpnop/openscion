//! The SQLite-backed metadata store.

use std::path::Path;
use std::sync::{Mutex, PoisonError};

use openscion_proto::{Changeset, ChangesetInfo, ObjectId, RefValue};
use rusqlite::{Connection, OptionalExtension, params};

use crate::{Error, Result};

use super::{Ancestry, MetadataStore, NewChangeset, StoredChangeset};

/// The schema: one append-only `changesets` table whose `AUTOINCREMENT` primary
/// key is the log sequence number, and one `refs` table with one row per ref.
/// Each changeset keeps its generation and parents as columns so an ancestry
/// walk reads them without decoding the whole changeset, and the full changeset
/// as a JSON payload. The first-parent skip list is a further column added by a
/// guarded migration in [`SqliteMetadataStore::from_connection`].
const SCHEMA: &str = "\
CREATE TABLE IF NOT EXISTS changesets (
    lsn        INTEGER PRIMARY KEY AUTOINCREMENT,
    id         TEXT NOT NULL UNIQUE,
    root_tree  TEXT NOT NULL,
    generation INTEGER NOT NULL,
    parents    TEXT NOT NULL,
    payload    TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS refs (
    name    TEXT PRIMARY KEY,
    target  TEXT NOT NULL,
    version INTEGER NOT NULL
);";

/// A metadata store backed by a single SQLite database.
///
/// The connection is wrapped in a [`Mutex`] because a SQLite connection is
/// `Send` but not `Sync`; a connection pool would allow concurrent readers, but
/// one serialized connection is correct and simple. The database runs in WAL
/// mode with `synchronous=FULL`, so a committed changeset or ref move survives a
/// power loss, and a ref compare-and-swap is one guarded `UPDATE` — a
/// single-record operation, matching the shardable design.
#[derive(Debug)]
pub struct SqliteMetadataStore {
    conn: Mutex<Connection>,
}

impl SqliteMetadataStore {
    /// Opens (creating if needed) a metadata store at `path`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Database`] when the database cannot be opened or the
    /// schema cannot be created.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::from_connection(Connection::open(path)?)
    }

    /// Opens an in-memory metadata store, for tests and ephemeral use.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Database`] when the database cannot be created.
    pub fn in_memory() -> Result<Self> {
        Self::from_connection(Connection::open_in_memory()?)
    }

    fn from_connection(conn: Connection) -> Result<Self> {
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = FULL;
             PRAGMA busy_timeout = 5000;",
        )?;
        conn.execute_batch(SCHEMA)?;
        // Add the first-parent skip-list column. Done as a guarded `ALTER`, not
        // in `SCHEMA`, so a fresh database, a reopen, and a database written
        // before skip lists existed all converge on the same shape; a row
        // written without it (the engine always writes it) defaults to empty.
        match conn.execute(
            "ALTER TABLE changesets ADD COLUMN skips TEXT NOT NULL DEFAULT '[]'",
            [],
        ) {
            Ok(_) => {}
            Err(error) if error.to_string().contains("duplicate column name") => {}
            Err(error) => return Err(error.into()),
        }
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Reads the current target of `name` with a held connection, for building
    /// a conflict error.
    fn ref_target(conn: &Connection, name: &str) -> Result<Option<ObjectId>> {
        conn.query_row("SELECT target FROM refs WHERE name = ?1", [name], |row| {
            row.get::<_, String>(0)
        })
        .optional()?
        .map(|text| parse_id(&text))
        .transpose()
    }
}

impl MetadataStore for SqliteMetadataStore {
    fn append(&mut self, new: NewChangeset) -> Result<ChangesetInfo> {
        let payload = serde_json::to_string(&new.changeset)?;
        let parents = serde_json::to_string(&new.changeset.parents)?;
        let skips = serde_json::to_string(&new.skips)?;
        let lsn: i64 = self.lock().query_row(
            "INSERT INTO changesets (id, root_tree, generation, parents, payload, skips)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6) RETURNING lsn",
            params![
                new.id.to_string(),
                new.root_tree.to_string(),
                i64::try_from(new.generation).unwrap_or(i64::MAX),
                parents,
                payload,
                skips,
            ],
            |row| row.get(0),
        )?;
        Ok(ChangesetInfo::new(
            new.id,
            u64::try_from(lsn).unwrap_or(0),
            new.root_tree,
        ))
    }

    fn info(&self, id: ObjectId) -> Result<Option<ChangesetInfo>> {
        let row = self
            .lock()
            .query_row(
                "SELECT lsn, root_tree FROM changesets WHERE id = ?1",
                [id.to_string()],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?;
        row.map(|(lsn, root_tree)| {
            Ok(ChangesetInfo::new(
                id,
                u64::try_from(lsn).unwrap_or(0),
                parse_id(&root_tree)?,
            ))
        })
        .transpose()
    }

    fn ancestry(&self, id: ObjectId) -> Result<Option<Ancestry>> {
        let row = self
            .lock()
            .query_row(
                "SELECT generation, parents FROM changesets WHERE id = ?1",
                [id.to_string()],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?;
        row.map(|(generation, parents)| {
            Ok(Ancestry {
                generation: u64::try_from(generation).unwrap_or(0),
                parents: serde_json::from_str(&parents)?,
            })
        })
        .transpose()
    }

    fn first_parent_skips(&self, id: ObjectId) -> Result<Option<Vec<ObjectId>>> {
        let skips = self
            .lock()
            .query_row(
                "SELECT skips FROM changesets WHERE id = ?1",
                [id.to_string()],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        skips
            .map(|skips| Ok(serde_json::from_str(&skips)?))
            .transpose()
    }

    fn record(&self, id: ObjectId) -> Result<Option<StoredChangeset>> {
        let row = self
            .lock()
            .query_row(
                "SELECT lsn, root_tree, generation, payload, skips FROM changesets WHERE id = ?1",
                [id.to_string()],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                    ))
                },
            )
            .optional()?;
        row.map(|(lsn, root_tree, generation, payload, skips)| {
            let changeset: Changeset = serde_json::from_str(&payload)?;
            Ok(StoredChangeset {
                info: ChangesetInfo::new(
                    id,
                    u64::try_from(lsn).unwrap_or(0),
                    parse_id(&root_tree)?,
                ),
                changeset,
                generation: u64::try_from(generation).unwrap_or(0),
                skips: serde_json::from_str(&skips)?,
            })
        })
        .transpose()
    }

    fn log_len(&self) -> Result<u64> {
        let count: i64 = self
            .lock()
            .query_row("SELECT COUNT(*) FROM changesets", [], |row| row.get(0))?;
        Ok(u64::try_from(count).unwrap_or(0))
    }

    fn get_ref(&self, name: &str) -> Result<Option<RefValue>> {
        let row = self
            .lock()
            .query_row(
                "SELECT target, version FROM refs WHERE name = ?1",
                [name],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
            )
            .optional()?;
        row.map(|(target, version)| {
            Ok(RefValue::new(
                name,
                parse_id(&target)?,
                u64::try_from(version).unwrap_or(0),
            ))
        })
        .transpose()
    }

    fn update_ref(
        &mut self,
        name: &str,
        expected: Option<ObjectId>,
        new: ObjectId,
    ) -> Result<RefValue> {
        let conn = self.lock();
        // Each arm is a single guarded statement, atomic in autocommit: a
        // matched row returns its new version, no match returns no rows.
        let version: rusqlite::Result<i64> = match expected {
            None => conn.query_row(
                "INSERT INTO refs (name, target, version) VALUES (?1, ?2, 1)
                 ON CONFLICT(name) DO NOTHING RETURNING version",
                params![name, new.to_string()],
                |row| row.get(0),
            ),
            Some(expected) => conn.query_row(
                "UPDATE refs SET target = ?1, version = version + 1
                 WHERE name = ?2 AND target = ?3 RETURNING version",
                params![new.to_string(), name, expected.to_string()],
                |row| row.get(0),
            ),
        };
        match version {
            Ok(version) => Ok(RefValue::new(
                name,
                new,
                u64::try_from(version).unwrap_or(0),
            )),
            Err(rusqlite::Error::QueryReturnedNoRows) => Err(Error::RefConflict {
                name: name.to_owned(),
                current: Self::ref_target(&conn, name)?,
            }),
            Err(error) => Err(error.into()),
        }
    }
}

impl From<rusqlite::Error> for Error {
    fn from(error: rusqlite::Error) -> Self {
        Self::Database {
            message: error.to_string(),
        }
    }
}

impl From<serde_json::Error> for Error {
    fn from(error: serde_json::Error) -> Self {
        Self::Database {
            message: error.to_string(),
        }
    }
}

/// Parses an object id stored as text, reporting corruption as a database error.
fn parse_id(text: &str) -> Result<ObjectId> {
    text.parse().map_err(|error| Error::Database {
        message: format!("stored id {text:?} is invalid: {error}"),
    })
}
