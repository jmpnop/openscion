//! The framed transport: a tier's object RPCs over a byte stream.
//!
//! Each message is a one-byte tag, an eight-byte big-endian body length, and
//! that many body bytes — the length-framing the engine uses to stream objects
//! (`FrameWriter`), carried over a socket. The object-get request's body is a
//! 32-byte [`ObjectId`]; a found response's body is the raw object bytes, so an
//! object never pays a text encoding on the wire. The decision to frame
//! `openscion-proto` values directly, over a Unix socket locally and TCP
//! between machines, is [ADR 0004](../../../../docs/adr/0004-daemon-transport.md).
//!
//! The codec, [`serve_connection`], and the client's request logic are generic
//! over any [`Read`] + [`Write`] stream and so are platform-independent; only
//! the Unix-socket listener and dialer are `#[cfg(unix)]`. Each accepted
//! connection is served on its own thread; a connection carries any number of
//! requests until the peer closes it.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::Arc;

#[cfg(unix)]
use std::os::unix::net::{UnixListener, UnixStream};

use openscion_core::ObjectId;

use crate::backing::ObjectBacking;
use crate::source::ObjectSource;
use crate::store::Store;
use crate::{Error, Result};

/// Request: get the object with this id. Body is the 32-byte id.
const TAG_GET: u8 = 1;
/// Response: the object was found. Body is its raw bytes.
const TAG_FOUND: u8 = 2;
/// Response: no tier holds the object. Body is empty.
const TAG_ABSENT: u8 = 3;
/// Response: the request failed. Body is a UTF-8 message.
const TAG_ERROR: u8 = 4;
/// Response: this tier shed the request (its upstream-fetch bound was reached).
/// Body is a UTF-8 reason. Distinct from [`TAG_ERROR`] so a lower tier can tell
/// a shed from a failure and back off instead of retrying.
const TAG_UNAVAILABLE: u8 = 5;

/// The largest body the codec will read, to bound the memory a single frame can
/// demand from a peer. Objects beyond this are chunked by the data model.
const MAX_BODY: u64 = 1 << 32;

/// Serves object RPCs from `store` over one connection until the peer closes
/// it. Generic over the stream, so it serves a Unix socket and a TCP connection
/// the same way.
///
/// # Errors
///
/// Returns [`Error::Protocol`] on a malformed frame and [`Error::Transport`] on
/// a read or write failure; a clean close is `Ok(())`.
pub fn serve_connection<St, S, B>(mut stream: St, store: &Store<S, B>) -> Result<()>
where
    St: Read + Write,
    S: ObjectSource,
    B: ObjectBacking,
{
    while let Some((tag, body)) = read_frame(&mut stream)? {
        if tag != TAG_GET {
            return Err(Error::Protocol(format!("unexpected request tag {tag}")));
        }
        let id = id_from(&body)?;
        match store.get(id) {
            Ok(bytes) => write_frame(&mut stream, TAG_FOUND, &bytes)?,
            Err(Error::NotFound(_)) => write_frame(&mut stream, TAG_ABSENT, &[])?,
            Err(Error::Unavailable(reason)) => {
                write_frame(&mut stream, TAG_UNAVAILABLE, reason.as_bytes())?;
            }
            Err(error) => write_frame(&mut stream, TAG_ERROR, error.to_string().as_bytes())?,
        }
    }
    Ok(())
}

/// Accepts connections on `listener` and serves each from `store` on its own
/// thread, until accepting fails.
///
/// # Errors
///
/// Returns [`Error::Transport`] when accepting a connection fails.
pub fn serve_tcp<S, B>(listener: &TcpListener, store: &Arc<Store<S, B>>) -> Result<()>
where
    S: ObjectSource + 'static,
    B: ObjectBacking + 'static,
{
    for stream in listener.incoming() {
        let stream = stream.map_err(Error::Transport)?;
        let store = Arc::clone(store);
        std::thread::spawn(move || {
            let _ = serve_connection(stream, &store);
        });
    }
    Ok(())
}

/// Accepts connections on a Unix-domain `listener` and serves each from `store`
/// on its own thread, until accepting fails.
///
/// # Errors
///
/// As [`serve_tcp`].
#[cfg(unix)]
pub fn serve_unix<S, B>(listener: &UnixListener, store: &Arc<Store<S, B>>) -> Result<()>
where
    S: ObjectSource + 'static,
    B: ObjectBacking + 'static,
{
    for stream in listener.incoming() {
        let stream = stream.map_err(Error::Transport)?;
        let store = Arc::clone(store);
        std::thread::spawn(move || {
            let _ = serve_connection(stream, &store);
        });
    }
    Ok(())
}

/// A client for a tier's object RPCs, reachable over a Unix socket or TCP.
///
/// It implements [`ObjectSource`], so a store on one tier can use a server on
/// the tier above as its upstream — tiers compose over the same wire. Each
/// fetch opens a fresh connection.
#[derive(Debug, Clone)]
pub struct SocketSource {
    endpoint: Endpoint,
}

/// Where a [`SocketSource`] dials.
#[derive(Debug, Clone)]
enum Endpoint {
    Tcp(SocketAddr),
    #[cfg(unix)]
    Unix(PathBuf),
}

impl SocketSource {
    /// A client that dials the server listening at `addr` over TCP.
    #[must_use]
    pub fn tcp(addr: SocketAddr) -> Self {
        Self {
            endpoint: Endpoint::Tcp(addr),
        }
    }

    /// A client that dials the server listening on the Unix socket at `path`.
    #[cfg(unix)]
    #[must_use]
    pub fn unix(path: impl Into<PathBuf>) -> Self {
        Self {
            endpoint: Endpoint::Unix(path.into()),
        }
    }

    /// Opens a connection to the endpoint.
    fn dial(&self) -> Result<Conn> {
        match &self.endpoint {
            Endpoint::Tcp(addr) => Ok(Conn::Tcp(
                TcpStream::connect(addr).map_err(Error::Transport)?,
            )),
            #[cfg(unix)]
            Endpoint::Unix(path) => Ok(Conn::Unix(
                UnixStream::connect(path).map_err(Error::Transport)?,
            )),
        }
    }
}

impl ObjectSource for SocketSource {
    fn fetch(&self, id: ObjectId) -> Result<Option<Arc<[u8]>>> {
        let mut conn = self.dial()?;
        write_frame(&mut conn, TAG_GET, id.as_bytes())?;
        match read_frame(&mut conn)? {
            Some((TAG_FOUND, body)) => Ok(Some(Arc::from(body))),
            Some((TAG_ABSENT, _)) => Ok(None),
            Some((TAG_UNAVAILABLE, reason)) => Err(Error::Unavailable(
                String::from_utf8_lossy(&reason).into_owned(),
            )),
            Some((TAG_ERROR, message)) => Err(Error::Upstream(
                String::from_utf8_lossy(&message).into_owned(),
            )),
            Some((tag, _)) => Err(Error::Protocol(format!("unexpected response tag {tag}"))),
            None => Err(Error::Protocol(
                "the peer closed before replying".to_owned(),
            )),
        }
    }
}

/// A connection to a server, over whichever transport the endpoint named.
enum Conn {
    Tcp(TcpStream),
    #[cfg(unix)]
    Unix(UnixStream),
}

impl Read for Conn {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Conn::Tcp(stream) => stream.read(buf),
            #[cfg(unix)]
            Conn::Unix(stream) => stream.read(buf),
        }
    }
}

impl Write for Conn {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            Conn::Tcp(stream) => stream.write(buf),
            #[cfg(unix)]
            Conn::Unix(stream) => stream.write(buf),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Conn::Tcp(stream) => stream.flush(),
            #[cfg(unix)]
            Conn::Unix(stream) => stream.flush(),
        }
    }
}

/// The id a `TAG_GET` body carries, or [`Error::Protocol`] when it is not a
/// 32-byte id.
fn id_from(body: &[u8]) -> Result<ObjectId> {
    let bytes: [u8; ObjectId::LEN] = body
        .try_into()
        .map_err(|_| Error::Protocol(format!("get body is {} bytes, not an id", body.len())))?;
    Ok(ObjectId::from_bytes(bytes))
}

/// Writes one framed message: tag, body length, body — then flushes.
fn write_frame<W: Write>(writer: &mut W, tag: u8, body: &[u8]) -> Result<()> {
    (|| {
        writer.write_all(&[tag])?;
        writer.write_all(&(body.len() as u64).to_be_bytes())?;
        writer.write_all(body)?;
        writer.flush()
    })()
    .map_err(Error::Transport)
}

/// Reads one framed message, or `None` at a clean end of stream.
fn read_frame<R: Read>(reader: &mut R) -> Result<Option<(u8, Vec<u8>)>> {
    let mut tag = [0u8; 1];
    match reader.read(&mut tag) {
        Ok(0) => return Ok(None),
        Ok(_) => {}
        Err(error) => return Err(Error::Transport(error)),
    }
    let mut length = [0u8; 8];
    read_exact(reader, &mut length)?;
    let length = u64::from_be_bytes(length);
    if length > MAX_BODY {
        return Err(Error::Protocol(format!(
            "frame body of {length} bytes exceeds the maximum"
        )));
    }
    let length = usize::try_from(length)
        .map_err(|_| Error::Protocol("frame body does not fit in memory".to_owned()))?;
    let mut body = vec![0u8; length];
    read_exact(reader, &mut body)?;
    Ok(Some((tag[0], body)))
}

/// Reads exactly `buf.len()` bytes, mapping a short read to a protocol error
/// (a truncated frame) and any other failure to a transport error.
fn read_exact<R: Read>(reader: &mut R, buf: &mut [u8]) -> Result<()> {
    reader.read_exact(buf).map_err(|error| {
        if error.kind() == std::io::ErrorKind::UnexpectedEof {
            Error::Protocol("truncated frame".to_owned())
        } else {
            Error::Transport(error)
        }
    })
}

#[cfg(test)]
mod test;
