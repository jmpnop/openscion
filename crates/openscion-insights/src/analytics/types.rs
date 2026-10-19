//! The analytics result rows.

/// A path and how many changes across history touched it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathChurn {
    /// The path, relative to the repository root.
    pub path: String,
    /// How many changes touched it.
    pub changes: u64,
}

/// An author and how many changesets they wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorActivity {
    /// Who wrote them.
    pub author: String,
    /// How many changesets they wrote.
    pub changesets: u64,
}
