//! The sparse set: which paths of a snapshot a working copy materializes.
//!
//! A working copy is either a [`Sparse::Full`] checkout — the whole tree — or a
//! [`Sparse::Cone`] of path prefixes, each a directory or file, and only the
//! paths under them are on disk. One rule, [`Sparse::covers`], decides whether a
//! path belongs, so clone, `sparse add`, and `sparse remove` agree.
//!
//! The set is normalized: a cone never holds a duplicate or an entry nested
//! under another, so each materialized path belongs for exactly one reason. The
//! serde form is backward compatible — a lone string (the shape before the set
//! was a set) loads as a one-entry cone, and null or an absent field as full.

use serde::de::Deserializer;
use serde::ser::Serializer;
use serde::{Deserialize, Serialize};

/// Which paths of the tree a working copy has on disk.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) enum Sparse {
    /// The whole tree.
    #[default]
    Full,
    /// Only the paths under these prefixes — a normalized, duplicate-free set
    /// with no entry nested under another. An empty cone materializes nothing.
    Cone(Vec<String>),
}

impl Sparse {
    /// A cone of the given prefixes, normalized.
    pub(crate) fn cone(prefixes: impl IntoIterator<Item = String>) -> Self {
        let mut sparse = Self::Cone(Vec::new());
        for prefix in prefixes {
            sparse.insert(prefix);
        }
        sparse
    }

    /// Whether this is a full checkout.
    pub(crate) fn is_full(&self) -> bool {
        matches!(self, Self::Full)
    }

    /// The cone's prefixes, sorted; empty for a full checkout.
    pub(crate) fn prefixes(&self) -> &[String] {
        match self {
            Self::Full => &[],
            Self::Cone(prefixes) => prefixes,
        }
    }

    /// Whether `path` is materialized: always for a full checkout, otherwise
    /// when it equals or lies under one of the cone's prefixes. This is the one
    /// membership rule clone, add, and remove share.
    pub(crate) fn covers(&self, path: &str) -> bool {
        match self {
            Self::Full => true,
            Self::Cone(prefixes) => prefixes.iter().any(|prefix| under(path, prefix)),
        }
    }

    /// Adds `prefix` to a cone, keeping it a normalized antichain: a no-op when
    /// `prefix` is already covered, and dropping any entries nested under it. A
    /// no-op on a full checkout.
    pub(crate) fn insert(&mut self, prefix: String) {
        if let Self::Cone(prefixes) = self {
            if prefixes.iter().any(|existing| under(&prefix, existing)) {
                return;
            }
            prefixes.retain(|existing| !under(existing, &prefix));
            prefixes.push(prefix);
            prefixes.sort();
        }
    }

    /// Removes an exact entry from a cone, returning whether it was present. A
    /// path that is not itself an entry (only covered by a broader one) is not
    /// removed.
    pub(crate) fn remove(&mut self, prefix: &str) -> bool {
        if let Self::Cone(prefixes) = self {
            let before = prefixes.len();
            prefixes.retain(|existing| existing != prefix);
            return prefixes.len() != before;
        }
        false
    }
}

/// Whether `path` equals `prefix` or lies under it (`prefix/…`).
fn under(path: &str, prefix: &str) -> bool {
    path == prefix
        || path
            .strip_prefix(prefix)
            .is_some_and(|rest| rest.starts_with('/'))
}

impl Serialize for Sparse {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Full => serializer.serialize_none(),
            Self::Cone(prefixes) => prefixes.serialize(serializer),
        }
    }
}

/// The legacy or current wire shapes the field may hold: a lone directory
/// string, or the set as an array.
#[derive(Deserialize)]
#[serde(untagged)]
enum Raw {
    One(String),
    Many(Vec<String>),
}

impl<'de> Deserialize<'de> for Sparse {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(match Option::<Raw>::deserialize(deserializer)? {
            None => Self::Full,
            Some(Raw::One(prefix)) => Self::cone([prefix]),
            Some(Raw::Many(prefixes)) => Self::cone(prefixes),
        })
    }
}
