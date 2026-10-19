//! Crate-wide error and result types.
//!
//! Every fallible public function in this crate returns [`Result`], and every
//! failure mode is a distinct [`Error`] variant. Add a variant rather than
//! encoding new context into an existing message: callers match on variants,
//! and message text is not a stable API.
//!
//! Variants carry the data a caller needs to react, keep their `#[error]`
//! message lowercase and free of trailing punctuation, and are documented so
//! the rendered rustdoc explains when each one occurs. [`Error::code`] gives
//! the stable name a server puts on the wire.

use openscion_proto::{ObjectId, PathError};

/// Errors returned by this crate.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// A path or ref name breaks the path rules.
    #[error(transparent)]
    InvalidPath(#[from] PathError),
    /// A changeset breaks one of the validity rules.
    #[error("invalid change at {path:?}: {rule}")]
    InvalidChange {
        /// The offending path, or empty when the rule is about the whole
        /// changeset.
        path: String,
        /// The rule it breaks.
        rule: &'static str,
    },
    /// No object or changeset has this id.
    #[error("object {0} not found")]
    NotFound(ObjectId),
    /// No ref has this name.
    #[error("ref {0:?} not found")]
    NoSuchRef(String),
    /// A ref did not hold the value a compare-and-swap expected.
    #[error("ref {name:?} conflict: it points at {}", display_target(*.current))]
    RefConflict {
        /// The ref's name.
        name: String,
        /// What it actually points at, or `None` if it does not exist.
        current: Option<ObjectId>,
    },
    /// Uploaded bytes do not hash to the id they were sent with.
    #[error("object {expected} hashes to {actual}")]
    CorruptObject {
        /// The id the bytes were sent with.
        expected: ObjectId,
        /// The id they actually hash to.
        actual: ObjectId,
    },
    /// Bytes are not a valid encoding of what they claim to be.
    #[error("malformed {what}: {reason}")]
    Malformed {
        /// What was being decoded, such as `tree` or `frame`.
        what: &'static str,
        /// What is wrong with it.
        reason: &'static str,
    },
    /// A rebase, revert, or land found paths changed on both sides. It created
    /// nothing; the caller resolves the paths and retries.
    #[error("merge conflict on {} path(s)", .paths.len())]
    MergeConflict {
        /// Every conflicting path, sorted and deduplicated.
        paths: Vec<String>,
    },
    /// A backing store (the filesystem, or later a database) could not complete
    /// an operation. The kind is retained; the full OS error is logged, not
    /// surfaced, so the error stays comparable.
    #[error("storage error at {path}: {kind}")]
    Storage {
        /// The path or location the operation touched.
        path: String,
        /// The kind of I/O error.
        kind: std::io::ErrorKind,
    },
    /// A metadata store backed by a database could not complete an operation.
    /// The driver's message is kept as text so the error stays comparable.
    #[error("metadata store error: {message}")]
    Database {
        /// What the database driver reported.
        message: String,
    },
}

fn display_target(current: Option<ObjectId>) -> String {
    current.map_or_else(|| "nothing".to_owned(), |id| id.to_string())
}

impl Error {
    /// The variant's stable name, used as the `code` in an error body.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidPath(_) => "InvalidPath",
            Self::InvalidChange { .. } => "InvalidChange",
            Self::NotFound(_) | Self::NoSuchRef(_) => "NotFound",
            Self::RefConflict { .. } => "RefConflict",
            Self::CorruptObject { .. } => "CorruptObject",
            Self::Malformed { .. } => "Malformed",
            Self::MergeConflict { .. } => "MergeConflict",
            Self::Storage { .. } => "Storage",
            Self::Database { .. } => "Database",
        }
    }
}

/// The crate's standard result type.
///
/// Use this alias in public signatures instead of spelling out
/// `std::result::Result<T, Error>`.
pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod test;
