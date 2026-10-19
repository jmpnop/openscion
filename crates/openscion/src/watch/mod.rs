//! `opnscn watch`: render a working copy's pending changes, live.
//!
//! A long-running watch keeps a working copy's pending-change set current by
//! re-examining only the paths the operating system reports as changed, instead
//! of walking the whole tree on every look. Its answer is always identical to a
//! full [`opnscn status`](crate::commands::status): the first render is a full
//! scan, each later change is re-verified through the same classifier, and any
//! event the watcher cannot attribute to specific files forces a full rescan, so
//! a missed or coarse event costs a scan, never a wrong answer.
//!
//! The design keeps the operating system at arm's length. [`WatchSession`]
//! folds [`WatchEvent`]s into a carried-forward report and is pure and
//! deterministic; [`run`] drives it from a channel; [`render`] prints a block.
//! Only [`watch`] and [`translate`] touch `notify`, turning native events into
//! [`WatchEvent`]s.

use std::io::Write;
use std::path::Path;
use std::sync::mpsc::Receiver;

use notify::{RecursiveMode, Watcher};

use crate::workcopy;
use crate::{Error, Result};

mod types;
pub(crate) use types::{WatchEvent, WatchSession};

/// Watches the working copy at `root`, rendering its pending changes to `out`
/// once at startup and again after each burst of filesystem activity, until the
/// process is interrupted.
///
/// # Errors
///
/// Returns [`Error::NotAWorkingCopy`] when `root` is not a working copy,
/// [`Error::Io`] when `root` cannot be resolved, [`Error::Watch`] when the
/// filesystem watcher cannot be set up, and [`Error::Output`] when a render
/// cannot be written.
pub(crate) fn watch(root: &Path, out: &mut impl Write) -> Result<()> {
    let (watcher, receiver, mut session) = arm(root)?;
    let result = run(&receiver, &mut session, out);
    drop(watcher);
    result
}

/// Sets up a recursive watch on the working copy at `root`: opens the session,
/// creates the event channel, and starts a `notify` watcher whose callback
/// translates native events onto the channel. Returns the watcher (which the
/// caller keeps alive for the watch's lifetime), the receiver, and the session.
///
/// # Errors
///
/// As [`watch`], minus the output error.
fn arm(
    root: &Path,
) -> Result<(
    notify::RecommendedWatcher,
    Receiver<WatchEvent>,
    WatchSession,
)> {
    let root = root.canonicalize().map_err(workcopy::io_error(root))?;
    let session = WatchSession::open(&root)?;
    let (sender, receiver) = std::sync::mpsc::channel();
    let watch_root = root.clone();
    let mut watcher = notify::recommended_watcher(move |result| {
        if let Some(event) = translate(&watch_root, result) {
            // The receiver lives as long as the watch; a send after it is gone
            // is harmless and ignored.
            let _ = sender.send(event);
        }
    })
    .map_err(|error| watch_error(&error))?;
    watcher
        .watch(&root, RecursiveMode::Recursive)
        .map_err(|error| watch_error(&error))?;
    Ok((watcher, receiver, session))
}

/// Renders once, then re-renders after each drained burst of events, until the
/// sender is dropped (the watcher stopped) or disconnects.
///
/// # Errors
///
/// Returns the working-copy error when a refresh fails, or [`Error::Output`]
/// when a render cannot be written.
fn run(
    events: &Receiver<WatchEvent>,
    session: &mut WatchSession,
    out: &mut impl Write,
) -> Result<()> {
    render(session, out)?;
    while let Ok(event) = events.recv() {
        session.apply(event);
        // Fold the rest of this burst before rendering, so a flurry of saves is
        // one render, not one per file.
        while let Ok(event) = events.try_recv() {
            session.apply(event);
        }
        render(session, out)?;
    }
    Ok(())
}

/// Writes the current pending change set — `A`/`M`/`D path` lines sorted by
/// path, or `clean` — then a blank line separating this render from the next,
/// and flushes so a reader of a pipe sees each render at once.
fn render(session: &mut WatchSession, out: &mut impl Write) -> Result<()> {
    let report = session.refresh()?;
    if report.is_empty() {
        writeln!(out, "clean").map_err(Error::Output)?;
    } else {
        for (path, code) in report {
            writeln!(out, "{} {path}", code.letter()).map_err(Error::Output)?;
        }
    }
    writeln!(out).map_err(Error::Output)?;
    out.flush().map_err(Error::Output)
}

/// Where a native event's path falls relative to the working-copy root.
#[derive(Debug, PartialEq, Eq)]
enum Relativized {
    /// A tracked location, as a `/`-separated path relative to the root.
    Inside(String),
    /// The root itself, or a path inside the `.opnscn` state directory: not a
    /// change to report, and not a reason to rescan.
    Ignored,
    /// Not under the root, or not representable: treat conservatively as a
    /// reason to rescan.
    Outside,
}

/// Places `path` relative to `root`. See [`Relativized`].
fn relativize(root: &Path, path: &Path) -> Relativized {
    let Ok(rel) = path.strip_prefix(root) else {
        return Relativized::Outside;
    };
    let mut parts = Vec::new();
    for component in rel.components() {
        let std::path::Component::Normal(part) = component else {
            return Relativized::Outside;
        };
        match part.to_str() {
            Some(part) => parts.push(part),
            None => return Relativized::Outside,
        }
    }
    match parts.first() {
        None => Relativized::Ignored,
        Some(&first) if first == workcopy::STATE_DIR => Relativized::Ignored,
        _ => Relativized::Inside(parts.join("/")),
    }
}

/// Turns a native watch event into a [`WatchEvent`], or `None` when it changes
/// nothing the working copy tracks (a read, or a `.opnscn` write).
///
/// Creates, removes, and content or metadata modifications of specific paths
/// become [`WatchEvent::Changed`]. An overflow, a rename, a path outside the
/// root, or any event whose effect cannot be pinned to named files becomes
/// [`WatchEvent::Rescan`], so the next render scans the whole tree rather than
/// trust an event it cannot attribute.
fn translate(root: &Path, result: notify::Result<notify::Event>) -> Option<WatchEvent> {
    use notify::EventKind;
    use notify::event::{Flag, ModifyKind};

    let Ok(event) = result else {
        return Some(WatchEvent::Rescan);
    };
    if event.flag() == Some(Flag::Rescan) {
        return Some(WatchEvent::Rescan);
    }
    match event.kind {
        EventKind::Access(_) => None,
        EventKind::Create(_)
        | EventKind::Remove(_)
        | EventKind::Modify(ModifyKind::Data(_) | ModifyKind::Metadata(_) | ModifyKind::Any) => {
            let mut paths = Vec::new();
            for path in &event.paths {
                match relativize(root, path) {
                    Relativized::Inside(rel) => paths.push(rel),
                    Relativized::Ignored => {}
                    Relativized::Outside => return Some(WatchEvent::Rescan),
                }
            }
            if paths.is_empty() {
                None
            } else {
                Some(WatchEvent::Changed(paths))
            }
        }
        // Renames, name-only modifies, and anything unknown: scan to be sure.
        _ => Some(WatchEvent::Rescan),
    }
}

/// Maps a `notify` failure to the client error.
fn watch_error(error: &notify::Error) -> Error {
    Error::Watch(error.to_string())
}

#[cfg(test)]
mod test;
