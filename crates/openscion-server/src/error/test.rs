//! Unit tests for the server error type.

use std::io;

use super::Error;

#[test]
fn a_bind_error_names_the_address() {
    let error = Error::Bind {
        addr: "127.0.0.1:7878".parse().unwrap_or_else(|_| unreachable!()),
        source: io::Error::from(io::ErrorKind::AddrInUse),
    };
    assert!(
        error
            .to_string()
            .starts_with("cannot listen on 127.0.0.1:7878: ")
    );
}

#[test]
fn a_serve_error_renders_its_cause() {
    let error = Error::Serve(io::Error::other("listener closed"));
    assert_eq!(error.to_string(), "server stopped: listener closed");
}

#[test]
fn a_store_error_names_its_cause() {
    let error = Error::Store(openscion_core::Error::Database {
        message: "disk full".to_owned(),
    });
    assert_eq!(
        error.to_string(),
        "cannot open the data store: metadata store error: disk full"
    );
}
