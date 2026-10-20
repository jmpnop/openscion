//! Unit tests for the git-ingestion error type: each variant's display and the
//! conversions from the engine's path and core errors.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use openscion_core::RepoPath;

use super::Error;

#[test]
fn a_git_error_displays_its_message() {
    let error = Error::Git("could not open repository".to_owned());
    assert_eq!(error.to_string(), "git: could not open repository");
}

#[test]
fn a_non_utf8_path_displays_its_bytes_lossily() {
    let error = Error::NonUtf8Path(b"bad-\xff-name".to_vec());
    assert_eq!(
        error.to_string(),
        "non-UTF-8 path in the git tree: bad-\u{fffd}-name"
    );
}

#[test]
fn a_path_error_converts_into_a_core_error() {
    // `RepoPath::new` rejects a `..` component with a `PathError`, which the
    // dedicated `From<PathError>` folds into `Error::Core`.
    let path_error = RepoPath::new("a/../b").unwrap_err();
    let error: Error = path_error.into();
    assert!(matches!(error, Error::Core(_)));
    // The transparent core error still renders a non-empty message.
    assert!(!error.to_string().is_empty());
}

#[test]
fn a_core_error_converts_through_the_from_impl() {
    let core_error: openscion_core::Error = RepoPath::new("a/../b").unwrap_err().into();
    let error = Error::from(core_error);
    assert!(matches!(error, Error::Core(_)));
}
