//! Error types of the link crate.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum LinkError {
    /// A Pro DJ Link port is taken – usually because rekordbox runs.
    #[error("UDP port {port} is in use ({source}) – is rekordbox running?")]
    PortInUse { port: u16, source: std::io::Error },
    #[error("network: {0}")]
    Io(#[from] std::io::Error),
    #[error("the link thread stopped")]
    Stopped,
}

/// A packet that could not be decoded.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PacketError {
    #[error("not a Pro DJ Link packet")]
    NoMagic,
    #[error("packet too short: {len} bytes, need {need}")]
    TooShort { len: usize, need: usize },
    #[error("unexpected packet type {0:#04x}")]
    WrongType(u8),
    #[error("{0}")]
    Format(String),
}
