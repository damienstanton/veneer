//! Errors are data. Each variant becomes a veneer `Law::Protocol` finding at the
//! CLI and MCP boundary.

use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NetError {
    /// Malformed input: an envelope, an op line, a flag.
    Invalid(String),
    /// The op is well-formed but the current state does not allow it.
    Rejected(String),
    /// Filesystem or subprocess failure.
    Io(String),
}

impl fmt::Display for NetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NetError::Invalid(m) => write!(f, "invalid: {m}"),
            NetError::Rejected(m) => write!(f, "rejected: {m}"),
            NetError::Io(m) => write!(f, "io: {m}"),
        }
    }
}

impl std::error::Error for NetError {}

impl From<std::io::Error> for NetError {
    fn from(e: std::io::Error) -> Self {
        NetError::Io(e.to_string())
    }
}

pub use crate::laws::fnv64;

impl NetError {
    pub fn finding(&self, at: &str) -> crate::laws::Finding {
        crate::laws::Finding::error(crate::laws::Law::Protocol, at, None, &self.to_string(), None)
    }
}
