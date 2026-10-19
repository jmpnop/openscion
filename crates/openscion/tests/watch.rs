//! Runs `opnscn watch` the way a user would: against a real working copy, with
//! a file appearing while it watches.
//!
//! This exercises the whole path — the `notify` backend, the event translation,
//! the session, and the rendered output — end to end through the binary. The
//! watch child is killed when the test is done, so its own coverage is not
//! collected; the in-process unit tests cover the logic, and this test confirms
//! the real filesystem watcher drives it.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use openscion_server::{Server, ServerConfig};

const BINARY: &str = env!("CARGO_BIN_EXE_opnscn");

/// Starts a real server on a free loopback port and returns its base URL.
fn real_server() -> String {
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let server = Server::bind(&ServerConfig::new("127.0.0.1:0".parse().unwrap()))
                .await
                .unwrap();
            sender.send(server.local_addr()).unwrap();
            server.serve(std::future::pending()).await.unwrap();
        });
    });
    format!("http://{}", receiver.recv().unwrap())
}

/// A fresh, empty scratch directory unique to this test run.
fn scratch() -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "openscion-watch-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Waits until a line containing `needle` arrives, or the deadline passes.
fn wait_for(lines: &mpsc::Receiver<String>, needle: &str, deadline: Instant) -> bool {
    while Instant::now() < deadline {
        match lines.recv_timeout(Duration::from_millis(200)) {
            Ok(line) if line.contains(needle) => return true,
            Ok(_) | Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => return false,
        }
    }
    false
}

#[test]
fn watch_renders_a_new_file_as_it_appears() {
    let base = real_server();
    let dir = scratch();
    std::fs::write(dir.join("seed.txt"), b"seed\n").unwrap();

    // Make the directory a working copy.
    let import = Command::new(BINARY)
        .args(["--server", &base, "import", dir.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        import.status.success(),
        "import failed: {}",
        String::from_utf8_lossy(&import.stderr)
    );

    // Start watching, reading its stdout line by line off a background thread.
    let mut child = Command::new(BINARY)
        .args(["watch", "--dir", dir.to_str().unwrap()])
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let stdout = child.stdout.take().unwrap();
    let (sender, lines) = mpsc::channel();
    let reader = std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    if sender.send(line.clone()).is_err() {
                        break;
                    }
                }
            }
        }
    });

    let deadline = Instant::now() + Duration::from_secs(30);
    assert!(
        wait_for(&lines, "clean", deadline),
        "the first render of a clean working copy should be `clean`"
    );

    // A new file appears while the watch is running.
    std::fs::write(dir.join("new.txt"), b"new\n").unwrap();
    assert!(
        wait_for(&lines, "A new.txt", deadline),
        "the watch should report the new file"
    );

    child.kill().unwrap();
    let _ = child.wait();
    drop(lines);
    let _ = reader.join();
    let _ = std::fs::remove_dir_all(&dir);
}
