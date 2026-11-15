//! The object id and object kind types.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// The content-derived name of a blob, tree, or changeset.
///
/// Two objects with the same id have the same kind and the same bytes. The
/// textual form is exactly 64 lowercase hexadecimal characters, and that is
/// also the JSON form.
///
/// # Examples
///
/// ```
/// # use openscion_proto::ObjectId;
/// let text = "00".repeat(31) + "ff";
/// let id: ObjectId = text.parse()?;
/// assert_eq!(id.as_bytes()[31], 0xff);
/// assert_eq!(id.to_string(), text);
/// # Ok::<(), openscion_proto::ParseIdError>(())
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ObjectId([u8; 32]);

impl ObjectId {
    /// The length of an id in bytes.
    pub const LEN: usize = 32;

    /// Wraps 32 raw hash bytes.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// The raw hash bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Display for ObjectId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut text = [0u8; 64];
        for (i, byte) in self.0.iter().enumerate() {
            text[2 * i] = HEX[usize::from(byte >> 4)];
            text[2 * i + 1] = HEX[usize::from(byte & 0x0f)];
        }
        // Every byte written above is an ASCII hex digit.
        f.write_str(std::str::from_utf8(&text).map_err(|_| fmt::Error)?)
    }
}

impl fmt::Debug for ObjectId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ObjectId({self})")
    }
}

impl FromStr for ObjectId {
    type Err = ParseIdError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let bytes = text.as_bytes();
        if bytes.len() != 2 * Self::LEN {
            return Err(ParseIdError::Length(bytes.len()));
        }
        let mut id = [0u8; 32];
        for (i, pair) in bytes.as_chunks::<2>().0.iter().enumerate() {
            id[i] = (nibble(pair[0])? << 4) | nibble(pair[1])?;
        }
        Ok(Self(id))
    }
}

fn nibble(digit: u8) -> Result<u8, ParseIdError> {
    match digit {
        b'0'..=b'9' => Ok(digit - b'0'),
        b'a'..=b'f' => Ok(digit - b'a' + 10),
        _ => Err(ParseIdError::Digit(char::from(digit))),
    }
}

impl Serialize for ObjectId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for ObjectId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

/// Why a string is not a valid [`ObjectId`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ParseIdError {
    /// The string is not exactly 64 characters long; carries its length.
    Length(usize),
    /// The string holds a character that is not a lowercase hex digit.
    Digit(char),
}

impl fmt::Display for ParseIdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Length(len) => write!(f, "object id must be 64 characters, got {len}"),
            Self::Digit(digit) => {
                write!(f, "object id holds {digit:?}, not a lowercase hex digit")
            }
        }
    }
}

impl std::error::Error for ParseIdError {}

/// What an object is. The kind is bound into the object's id, so a blob and a
/// tree with the same bytes still have different ids.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObjectKind {
    /// A file's contents.
    Blob,
    /// One directory's entries.
    Tree,
    /// A unit of history.
    Changeset,
}

impl ObjectKind {
    /// The kind's name as it appears in the hashed header.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Blob => "blob",
            Self::Tree => "tree",
            Self::Changeset => "changeset",
        }
    }
}
