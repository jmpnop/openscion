//! The listen/dial endpoint type: a scheme-prefixed TCP address or Unix path.
//!
//! An [`Endpoint`] names where a tier accepts connections or dials the tier
//! above, parsed from one scheme-prefixed string so a single operator-facing
//! flag spans every tier. It lives here, beside the [transport](crate::transport)
//! it addresses, so the daemon, the cache, and the server share one parser
//! rather than each carrying its own. [`dial`](Endpoint::dial) bridges an
//! endpoint to the [`SocketSource`] that reads through the tier above it.

use std::fmt;
use std::net::{SocketAddr, TcpListener};
#[cfg(unix)]
use std::os::unix::net::UnixListener;
#[cfg(unix)]
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;

use crate::service::ObjectService;
#[cfg(unix)]
use crate::transport::serve_unix;
use crate::transport::{SocketSource, serve_tcp};

/// Where a tier accepts connections, or dials a tier above.
///
/// Parsed from a scheme-prefixed string — `tcp:<addr>` or `unix:<path>` — so one
/// flag names either transport unambiguously. [`Display`](fmt::Display) renders
/// the same form, so a resolved address round-trips back to the operator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Endpoint {
    /// A TCP socket address, from `tcp:<addr>` (e.g. `tcp:127.0.0.1:7979` or
    /// `tcp:[::1]:7979`).
    Tcp(SocketAddr),
    /// A Unix-domain socket path, from `unix:<path>`.
    #[cfg(unix)]
    Unix(PathBuf),
}

impl Endpoint {
    /// A [`SocketSource`] that reads through the tier above at this endpoint, so
    /// a store on one tier fetches from the tier above over the framed wire.
    #[must_use]
    pub fn dial(&self) -> SocketSource {
        match self {
            Self::Tcp(addr) => SocketSource::tcp(*addr),
            #[cfg(unix)]
            Self::Unix(path) => SocketSource::unix(path.clone()),
        }
    }

    /// Binds a listener on this endpoint — the server side of the wire,
    /// symmetric with [`dial`](Self::dial).
    ///
    /// # Errors
    ///
    /// Returns the bind's [`io::Error`](std::io::Error), which the caller wraps
    /// in its own error type.
    pub fn bind(&self) -> std::io::Result<BoundListener> {
        match self {
            Self::Tcp(addr) => {
                let listener = TcpListener::bind(addr)?;
                let listening = format!("tcp:{}", listener.local_addr()?);
                Ok(BoundListener {
                    inner: Inner::Tcp(listener),
                    listening,
                })
            }
            #[cfg(unix)]
            Self::Unix(path) => {
                let listener = UnixListener::bind(path)?;
                Ok(BoundListener {
                    inner: Inner::Unix(listener),
                    listening: format!("unix:{}", path.display()),
                })
            }
        }
    }
}

/// The server side of an [`Endpoint`]: a bound listener ready to
/// [`serve`](BoundListener::serve).
///
/// [`Endpoint::bind`] produces one, resolving any port-0 to the port the OS
/// chose, so a tier reports where it actually listens. It hides the TCP/Unix
/// split: a tier binds, reports [`listening`](BoundListener::listening), and
/// serves without a `#[cfg]` of its own.
#[derive(Debug)]
pub struct BoundListener {
    inner: Inner,
    listening: String,
}

/// The bound listener, over whichever transport the endpoint named.
#[derive(Debug)]
enum Inner {
    Tcp(TcpListener),
    #[cfg(unix)]
    Unix(UnixListener),
}

impl BoundListener {
    /// The resolved `tcp:<addr>`/`unix:<path>` this is listening on — for
    /// `tcp:…:0`, the port the OS chose.
    #[must_use]
    pub fn listening(&self) -> &str {
        &self.listening
    }

    /// Serves `service` on this listener, each connection on its own thread,
    /// until accepting fails. Dispatches to the TCP or Unix transport.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Transport`](crate::Error::Transport) when accepting a
    /// connection fails.
    pub fn serve<Svc: ObjectService + 'static>(&self, service: &Arc<Svc>) -> crate::Result<()> {
        match &self.inner {
            Inner::Tcp(listener) => serve_tcp(listener, service),
            #[cfg(unix)]
            Inner::Unix(listener) => serve_unix(listener, service),
        }
    }
}

impl fmt::Display for Endpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Tcp(addr) => write!(f, "tcp:{addr}"),
            #[cfg(unix)]
            Self::Unix(path) => write!(f, "unix:{}", path.display()),
        }
    }
}

impl FromStr for Endpoint {
    type Err = EndpointParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.split_once(':') {
            Some(("tcp", addr)) => addr
                .parse()
                .map(Self::Tcp)
                .map_err(|_| EndpointParseError(format!("`{addr}` is not a TCP socket address"))),
            #[cfg(unix)]
            Some(("unix", path)) if !path.is_empty() => Ok(Self::Unix(PathBuf::from(path))),
            #[cfg(unix)]
            Some(("unix", _)) => Err(EndpointParseError(
                "a unix endpoint needs a path".to_owned(),
            )),
            #[cfg(not(unix))]
            Some(("unix", _)) => Err(EndpointParseError(
                "unix endpoints are not supported on this platform".to_owned(),
            )),
            _ => Err(EndpointParseError(format!(
                "`{value}` is not a tcp:<addr> or unix:<path> endpoint"
            ))),
        }
    }
}

/// Why a string is not a valid [`Endpoint`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EndpointParseError(String);

impl fmt::Display for EndpointParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for EndpointParseError {}

#[cfg(test)]
mod test;
