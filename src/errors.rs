//! Error types for the reconciler library.

use std::fmt;

/// Errors produced by ingestion and decoding.
#[derive(Debug)]
pub enum ReconcileError {
    /// The input could not be decoded as a settlement record.
    Decode(String),
    /// An I/O operation failed.
    Io(std::io::Error),
}

impl fmt::Display for ReconcileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Decode(msg) => write!(f, "decode error: {msg}"),
            Self::Io(err) => write!(f, "io error: {err}"),
        }
    }
}

impl std::error::Error for ReconcileError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(err) => Some(err),
            Self::Decode(_) => None,
        }
    }
}

impl From<std::io::Error> for ReconcileError {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err)
    }
}

/// Convenience alias used across the crate.
pub type Result<T> = std::result::Result<T, ReconcileError>;
