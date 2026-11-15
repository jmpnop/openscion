//! Runs the `openscion-cache` binary the way an operator would: it binds and
//! prints its listen line, answers a client over the wire by reading through a
//! running upstream, and a `SIGTERM` stops it cleanly.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::{BufRead, BufReader};
use std::net::{SocketAddr, TcpListener};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;

use openscion_cache::{CacheStore, MemoryBacking, ObjectSource, Result, SocketSource, serve_tcp};
use openscion_core::{ObjectId, ObjectKind, object_id};

const BINARY: &str = env!("CARGO_BIN_EXE_openscion-cache");

/// A tier-above holding exactly one object, served over the framed wire.
struct OneObject {
    id: ObjectId,
    bytes: Arc<[u8]>,
}

impl ObjectSource for OneObject {
    fn fetch(&self, id: ObjectId) -> Result<Option<Arc<[u8]>>> {
        Ok((id == self.id).then(|| Arc::clone(&self.bytes)))
    }
}

/// Stands up an in-process server over TCP holding `bytes` as a blob — a stand
/// -in for the tier above the cache — and returns its address and the object id.
fn serve_upstream(bytes: &[u8]) -> (SocketAddr, ObjectId) {
    let id = object_id(ObjectKind::Blob, bytes);
    let source = OneObject {
        id,
        bytes: Arc::from(bytes),
    };
    let store: Arc<CacheStore<OneObject>> = Arc::new(CacheStore::<OneObject>::with_backing(
        source,
        1 << 20,
        MemoryBacking::new(),
    ));
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        let _ = serve_tcp(&listener, &store);
    });
    (addr, id)
}

/// Reads the cache's `openscion-cache listening on <endpoint>` line.
fn listening_endpoint(child: &mut Child) -> String {
    let mut line = String::new();
    BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    line.trim()
        .strip_prefix("openscion-cache listening on ")
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
fn serves_by_reading_through_the_upstream_then_a_sigterm_exits() {
    let (upstream, id) = serve_upstream(b"a regional object");

    let upstream_arg = format!("tcp:{upstream}");
    let mut child = Command::new(BINARY)
        .args(["--upstream", &upstream_arg, "--listen", "tcp:127.0.0.1:0"])
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();

    let endpoint = listening_endpoint(&mut child);
    let addr = endpoint.strip_prefix("tcp:").unwrap().parse().unwrap();

    // A client of the cache reads the object; the cache misses, reads through
    // the upstream over the wire, and serves it.
    let client = SocketSource::tcp(addr);
    assert_eq!(&*client.fetch(id).unwrap().unwrap(), b"a regional object");
    // An id no tier holds comes back absent, not an error.
    assert!(
        client
            .fetch(object_id(ObjectKind::Blob, b"absent"))
            .unwrap()
            .is_none()
    );

    terminate(&mut child);
    assert!(child.wait().unwrap().success());
}

#[test]
fn exits_with_failure_when_the_address_is_taken() {
    let taken = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = taken.local_addr().unwrap();
    let listen_arg = format!("tcp:{addr}");

    let output = Command::new(BINARY)
        .args(["--upstream", "tcp:127.0.0.1:1", "--listen", &listen_arg])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.starts_with("openscion-cache: "),
        "stderr was {stderr:?}"
    );
}

#[test]
fn exits_with_failure_on_a_malformed_endpoint() {
    let output = Command::new(BINARY)
        .args(["--upstream", "tcp:127.0.0.1:1", "--listen", "ftp:nonsense"])
        .output()
        .unwrap();
    assert!(!output.status.success());
}

#[test]
fn exits_with_failure_when_no_upstream_is_given() {
    let output = Command::new(BINARY)
        .args(["--listen", "tcp:127.0.0.1:0"])
        .output()
        .unwrap();
    assert!(!output.status.success());
}
