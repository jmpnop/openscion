//! The listen/dial endpoint type: a scheme-prefixed TCP address or Unix path.

use std::fmt;
use std::net::SocketAddr;
#[cfg(unix)]
use std::path::PathBuf;
use std::str::FromStr;

/// Where the daemon accepts connections, or dials a tier above.
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
