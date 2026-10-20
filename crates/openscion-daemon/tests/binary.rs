//! Runs the `openscion-daemon` binary the way an operator would: it binds and
//! prints its listen line, answers a client over the wire, and a `SIGTERM`
//! stops it cleanly and writes the snapshot.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::{BufRead, BufReader};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

use openscion_core::ObjectId;
use openscion_daemon::{ObjectSource, SocketSource};

const BINARY: &str = env!("CARGO_BIN_EXE_openscion-daemon");

/// A fresh, unique scratch path under the temp directory for one test.
fn scratch(name: &str) -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let path = std::env::temp_dir().join(format!(
        "openscion-daemon-bin-{}-{name}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&path);
    path
}

/// Reads the daemon's `openscion-daemon listening on <endpoint>` line.
fn listening_endpoint(child: &mut Child) -> String {
    let mut line = String::new();
    BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    line.trim()
        .strip_prefix("openscion-daemon listening on ")
        .unwrap_or_else(|| panic!("unexpected first line: {line:?}"))
        .to_owned()
}

#[cfg(unix)]
fn terminate(child: &mut Child) {
    let killed = Command::new("kill")
        .args(["-TERM", &child.id().to_string()])
        .status()
        .unwrap();
    assert!(killed.success());
}

#[cfg(unix)]
#[test]
fn serves_over_tcp_then_a_sigterm_writes_the_snapshot_and_exits() {
    let data_dir = scratch("tcp-data");
    let snapshot = scratch("tcp-snap");
    let mut child = Command::new(BINARY)
        .args([
            "--data-dir".as_ref(),
            data_dir.as_os_str(),
            "--listen".as_ref(),
            "tcp:127.0.0.1:0".as_ref(),
            "--snapshot".as_ref(),
            snapshot.as_os_str(),
        ])
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();

    let endpoint = listening_endpoint(&mut child);
    let addr = endpoint.strip_prefix("tcp:").unwrap().parse().unwrap();

    // A leaf daemon holds nothing, so any object reads as absent over the wire.
    let source = SocketSource::tcp(addr);
    assert!(
        source
            .fetch(ObjectId::from_bytes([7; 32]))
            .unwrap()
            .is_none()
    );

    terminate(&mut child);
    assert!(child.wait().unwrap().success());
    assert!(Path::new(&snapshot).exists(), "snapshot was not written");
}

#[cfg(unix)]
#[test]
fn serves_over_a_unix_socket() {
    let data_dir = scratch("unix-data");
    let socket = scratch("unix-sock");
    let mut child = Command::new(BINARY)
        .args([
            "--data-dir".as_ref(),
            data_dir.as_os_str(),
            "--listen".as_ref(),
            format!("unix:{}", socket.display()).as_ref(),
        ])
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();

    let endpoint = listening_endpoint(&mut child);
    assert_eq!(endpoint, format!("unix:{}", socket.display()));

    let source = SocketSource::unix(&socket);
    assert!(
        source
            .fetch(ObjectId::from_bytes([9; 32]))
            .unwrap()
            .is_none()
    );

    terminate(&mut child);
    assert!(child.wait().unwrap().success());
}

#[test]
fn exits_with_failure_when_the_address_is_taken() {
    let taken = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = taken.local_addr().unwrap().to_string();

    let output = Command::new(BINARY)
        .args(["--data-dir", "/tmp", "--listen", &format!("tcp:{addr}")])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.starts_with("openscion-daemon: "),
        "stderr was {stderr:?}"
    );
}

#[test]
fn exits_with_failure_on_a_malformed_endpoint() {
    let output = Command::new(BINARY)
        .args(["--data-dir", "/tmp", "--listen", "ftp:nonsense"])
        .output()
        .unwrap();
    assert!(!output.status.success());
}
