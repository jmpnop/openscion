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
use std::sync::{Arc, Mutex, PoisonError};

#[cfg(unix)]
use std::os::unix::net::{UnixListener, UnixStream};

use openscion_core::ObjectId;
use openscion_core::rpc::{Request, Response as RpcResponse};

use crate::service::ObjectService;
use crate::source::ObjectSource;
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
/// Request: get many objects at once. Body is their 32-byte ids concatenated.
/// The reply is one response frame per id — `found`/`absent`/`unavailable`/
/// `error` — in request order.
const TAG_GET_MANY: u8 = 6;
/// Request: a forwarded non-object RPC. Body is a postcard-encoded
/// [`Request`](openscion_core::rpc::Request); the reply is [`TAG_RPC_REPLY`].
const TAG_RPC: u8 = 7;
/// Response: the answer to a [`TAG_RPC`] request. Body is a postcard-encoded
/// [`Response`](openscion_core::rpc::Response), including its error variant.
const TAG_RPC_REPLY: u8 = 8;

/// The largest body the codec will read, to bound the memory a single frame can
/// demand from a peer. Objects beyond this are chunked by the data model.
const MAX_BODY: u64 = 1 << 32;

/// Serves object RPCs from `service` over one connection until the peer closes
/// it. Generic over the stream, so it serves a Unix socket and a TCP connection
/// the same way, and over the [`ObjectService`], so it serves a cache
/// [`Store`](crate::Store) and an origin's authoritative store alike.
///
/// # Errors
///
/// Returns [`Error::Protocol`] on a malformed frame and [`Error::Transport`] on
/// a read or write failure; a clean close is `Ok(())`.
pub fn serve_connection<St, Svc>(mut stream: St, service: &Svc) -> Result<()>
where
    St: Read + Write,
    Svc: ObjectService + ?Sized,
{
    while let Some((tag, body)) = read_frame(&mut stream)? {
        match tag {
            TAG_GET => respond(&mut stream, service.get(id_from(&body)?))?,
            TAG_GET_MANY => {
                // One reply per id, in request order; the client reads exactly
                // this many.
                for id in ids_from(&body)? {
                    respond(&mut stream, service.get(id))?;
                }
            }
            TAG_RPC => {
                let request: Request = postcard::from_bytes(&body).map_err(|error| {
                    Error::Protocol(format!("decoding a framed rpc failed: {error}"))
                })?;
                let reply = postcard::to_allocvec(&service.rpc(request)?).map_err(|error| {
                    Error::Protocol(format!("encoding a framed rpc failed: {error}"))
                })?;
                write_frame(&mut stream, TAG_RPC_REPLY, &reply)?;
            }
            _ => return Err(Error::Protocol(format!("unexpected request tag {tag}"))),
        }
    }
    Ok(())
}

/// Writes the one response frame a `get` outcome maps to: the bytes when found,
/// an empty `absent`, an `unavailable` shed, or an `error` message.
fn respond<W: Write>(stream: &mut W, result: Result<Arc<[u8]>>) -> Result<()> {
    match result {
        Ok(bytes) => write_frame(stream, TAG_FOUND, &bytes),
        Err(Error::NotFound(_)) => write_frame(stream, TAG_ABSENT, &[]),
        Err(Error::Unavailable(reason)) => write_frame(stream, TAG_UNAVAILABLE, reason.as_bytes()),
        Err(error) => write_frame(stream, TAG_ERROR, error.to_string().as_bytes()),
    }
}

/// Accepts connections on `listener` and serves each from `service` on its own
/// thread, until accepting fails.
///
/// # Errors
///
/// Returns [`Error::Transport`] when accepting a connection fails.
pub fn serve_tcp<Svc>(listener: &TcpListener, service: &Arc<Svc>) -> Result<()>
where
    Svc: ObjectService + 'static,
{
    for stream in listener.incoming() {
        let stream = stream.map_err(Error::Transport)?;
        let service = Arc::clone(service);
        std::thread::spawn(move || {
            let _ = serve_connection(stream, &*service);
        });
    }
    Ok(())
}

/// Accepts connections on a Unix-domain `listener` and serves each from
/// `service` on its own thread, until accepting fails.
///
/// # Errors
///
/// As [`serve_tcp`].
#[cfg(unix)]
pub fn serve_unix<Svc>(listener: &UnixListener, service: &Arc<Svc>) -> Result<()>
where
    Svc: ObjectService + 'static,
{
    for stream in listener.incoming() {
        let stream = stream.map_err(Error::Transport)?;
        let service = Arc::clone(service);
        std::thread::spawn(move || {
            let _ = serve_connection(stream, &*service);
        });
    }
    Ok(())
}

/// A client for a tier's object RPCs, reachable over a Unix socket or TCP.
///
/// It implements [`ObjectSource`], so a store on one tier can use a server on
/// the tier above as its upstream — tiers compose over the same wire.
///
/// Connections are pooled and reused across fetches rather than dialed per
/// fetch, so a burst of reads amortizes one connect over many requests; a pooled
/// connection the peer closed while idle is retried once on a fresh one. The
/// pool is shared across clones, so the one source a store holds reuses
/// connections no matter how many threads fetch through it.
#[derive(Debug, Clone)]
pub struct SocketSource {
    endpoint: Endpoint,
    idle: Arc<Mutex<Vec<Conn>>>,
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
        Self::new(Endpoint::Tcp(addr))
    }

    /// A client that dials the server listening on the Unix socket at `path`.
    #[cfg(unix)]
    #[must_use]
    pub fn unix(path: impl Into<PathBuf>) -> Self {
        Self::new(Endpoint::Unix(path.into()))
    }

    /// A client over `endpoint` with an empty connection pool.
    fn new(endpoint: Endpoint) -> Self {
        Self {
            endpoint,
            idle: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Opens a fresh connection to the endpoint.
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

    /// Takes an idle connection from the pool, if any.
    fn take_idle(&self) -> Option<Conn> {
        self.idle
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop()
    }

    /// Returns a healthy connection to the pool for the next fetch. The pool is
    /// self-bounding: a connection enters it only after being checked out, so the
    /// idle count never exceeds the peak number of concurrent fetches.
    fn checkin(&self, conn: Conn) {
        self.idle
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(conn);
    }

    /// Runs `exchange` on a connection and returns a healthy one to the pool.
    ///
    /// A pooled connection is tried first; if the peer closed it while it sat
    /// idle, `exchange` fails at the transport and we retry once on a fresh
    /// connection — a stale pool costs a retry, not an error. A failure on a
    /// freshly dialed connection is a real error and is returned. `exchange`
    /// returns `Err` only when the connection is unusable, so it is never
    /// returned to the pool after a failure.
    fn with_connection<T>(&self, exchange: impl Fn(&mut Conn) -> Result<T>) -> Result<T> {
        if let Some(mut conn) = self.take_idle()
            && let Ok(value) = exchange(&mut conn)
        {
            self.checkin(conn);
            return Ok(value);
        }
        let mut conn = self.dial()?;
        let value = exchange(&mut conn)?;
        self.checkin(conn);
        Ok(value)
    }

    /// Sends a forwarded non-object [`Request`] and returns the server's
    /// [`Response`](openscion_core::rpc::Response), over a pooled connection —
    /// the client side of the mutable queries a lower tier forwards upstream.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Transport`]/[`Error::Protocol`] when the connection fails
    /// or replies malformed; an engine failure is carried in the response's error
    /// variant, returned as `Ok`.
    pub fn rpc(&self, request: &Request) -> Result<RpcResponse> {
        let body = postcard::to_allocvec(request)
            .map_err(|error| Error::Protocol(format!("encoding a framed rpc failed: {error}")))?;
        self.with_connection(|conn| rpc_exchange(conn, &body))
    }
}

impl ObjectSource for SocketSource {
    fn fetch(&self, id: ObjectId) -> Result<Option<Arc<[u8]>>> {
        self.with_connection(|conn| exchange(conn, id))?.outcome()
    }

    fn fetch_many(&self, ids: &[ObjectId]) -> Result<Vec<Option<Arc<[u8]>>>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let responses = self.with_connection(|conn| exchange_many(conn, ids))?;
        collect_outcomes(responses)
    }
}

/// A well-formed reply to a `get`, before it is mapped onto the caller's result.
/// A connection that produced one of these is healthy and can be reused.
enum Response {
    Found(Arc<[u8]>),
    Absent,
    Unavailable(String),
    Upstream(String),
}

impl Response {
    /// Maps a reply onto an [`ObjectSource::fetch`] result.
    fn outcome(self) -> Result<Option<Arc<[u8]>>> {
        match self {
            Response::Found(bytes) => Ok(Some(bytes)),
            Response::Absent => Ok(None),
            Response::Unavailable(reason) => Err(Error::Unavailable(reason)),
            Response::Upstream(message) => Err(Error::Upstream(message)),
        }
    }
}

/// Performs one `get` exchange on `conn`: writes the request, reads the reply.
///
/// Returns the [`Response`] on a complete exchange — the connection is then
/// healthy — and [`Error::Transport`]/[`Error::Protocol`] when the connection
/// failed or replied malformed, which means it is unusable and must be dropped.
fn exchange(conn: &mut Conn, id: ObjectId) -> Result<Response> {
    write_frame(conn, TAG_GET, id.as_bytes())?;
    read_response(conn)
}

/// Performs one `get-many` exchange on `conn`: writes the ids in one request and
/// reads one reply per id, in request order.
///
/// Returns `Err` only on a transport or protocol failure — a per-id *unavailable*
/// or *error* is carried in its [`Response`], not a connection fault — so the
/// connection is healthy whenever this returns `Ok`, even if some ids failed.
fn exchange_many(conn: &mut Conn, ids: &[ObjectId]) -> Result<Vec<Response>> {
    let mut body = Vec::with_capacity(ids.len() * ObjectId::LEN);
    for id in ids {
        body.extend_from_slice(id.as_bytes());
    }
    write_frame(conn, TAG_GET_MANY, &body)?;
    (0..ids.len()).map(|_| read_response(conn)).collect()
}

/// Reads one reply frame and maps it to a [`Response`], or to
/// [`Error::Transport`]/[`Error::Protocol`] when the connection failed or replied
/// malformed.
fn read_response(conn: &mut Conn) -> Result<Response> {
    match read_frame(conn)? {
        Some((TAG_FOUND, body)) => Ok(Response::Found(Arc::from(body))),
        Some((TAG_ABSENT, _)) => Ok(Response::Absent),
        Some((TAG_UNAVAILABLE, reason)) => Ok(Response::Unavailable(
            String::from_utf8_lossy(&reason).into_owned(),
        )),
        Some((TAG_ERROR, message)) => Ok(Response::Upstream(
            String::from_utf8_lossy(&message).into_owned(),
        )),
        Some((tag, _)) => Err(Error::Protocol(format!("unexpected response tag {tag}"))),
        None => Err(Error::Protocol(
            "the peer closed before replying".to_owned(),
        )),
    }
}

/// Maps a batch's replies onto fetch results, short-circuiting on the first id
/// that was unavailable or errored.
fn collect_outcomes(responses: Vec<Response>) -> Result<Vec<Option<Arc<[u8]>>>> {
    responses.into_iter().map(Response::outcome).collect()
}

/// Performs one forwarded-RPC exchange on `conn`: writes the encoded request and
/// reads the decoded reply.
///
/// Returns `Err` only on a transport or protocol failure — an engine failure is
/// carried in the decoded response's error variant — so the connection is
/// healthy whenever this returns `Ok`.
fn rpc_exchange(conn: &mut Conn, request_body: &[u8]) -> Result<RpcResponse> {
    write_frame(conn, TAG_RPC, request_body)?;
    match read_frame(conn)? {
        Some((TAG_RPC_REPLY, body)) => postcard::from_bytes(&body)
            .map_err(|error| Error::Protocol(format!("decoding a framed rpc failed: {error}"))),
        Some((tag, _)) => Err(Error::Protocol(format!("unexpected response tag {tag}"))),
        None => Err(Error::Protocol(
            "the peer closed before replying".to_owned(),
        )),
    }
}

/// A connection to a server, over whichever transport the endpoint named.
#[derive(Debug)]
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

/// The ids a `TAG_GET_MANY` body carries — its bytes as a whole number of 32-byte
/// ids — or [`Error::Protocol`] when its length is not a multiple of an id.
fn ids_from(body: &[u8]) -> Result<Vec<ObjectId>> {
    let (ids, rest) = body.as_chunks::<{ ObjectId::LEN }>();
    if !rest.is_empty() {
        return Err(Error::Protocol(format!(
            "get-many body of {} bytes is not a whole number of ids",
            body.len()
        )));
    }
    Ok(ids
        .iter()
        .map(|&bytes| ObjectId::from_bytes(bytes))
        .collect())
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
