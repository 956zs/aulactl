//! Error type shared by every module.

use std::fmt;

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// No WIN 60 HE vendor interface was found.
    NotFound,
    /// Opening, reading or writing the hidraw node or a file failed.
    Io(String),
    /// Another aulactl process holds the device for longer than the lock wait.
    Busy(String),
    /// The keyboard did not answer in time (protocol 2.3: 200 ms per frame, 1 s per table).
    Timeout(String),
    /// The keyboard answered with something this crate does not understand.
    Protocol(String),
    /// `0x0D` returned a product code other than `SI2825HEARGB` / `SI2825KZHEARGB` (protocol 1.1).
    /// Only identification commands are sent to such a board.
    UnsupportedProduct { product_code: String },
    /// The board's capability bits (`0x01`) say it lacks this feature.
    Unsupported(String),
    /// A value given by the caller is out of range or malformed.
    Invalid(String),
    /// A frame that is not on the allowlist of protocol 2.6 (or is marked DO NOT USE) was about to be sent.
    Refused(String),
    /// A written table read back different from what was written.
    Verify(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::NotFound => write!(f, "no AULA WIN 60 HE found (is it plugged in by USB?)"),
            Error::Io(m) | Error::Busy(m) | Error::Protocol(m) | Error::Invalid(m) => f.write_str(m),
            Error::Timeout(m) => write!(f, "timeout: {m}"),
            Error::UnsupportedProduct { product_code } => write!(
                f,
                "unsupported board (product code {product_code:?}); only SI2825HEARGB / SI2825KZHEARGB are supported"
            ),
            Error::Unsupported(m) => write!(f, "not supported by this board: {m}"),
            Error::Refused(m) => write!(f, "refused to send: {m}"),
            Error::Verify(m) => write!(f, "read-back mismatch: {m}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<Error> for String {
    fn from(e: Error) -> String {
        e.to_string()
    }
}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Error::Invalid(format!("JSON: {e}"))
    }
}

/// Shorthand for `Err(Error::Invalid(..))`.
pub(crate) fn invalid<T>(msg: impl Into<String>) -> Result<T> {
    Err(Error::Invalid(msg.into()))
}

/// Shorthand for `Err(Error::Protocol(..))`.
pub(crate) fn protocol<T>(msg: impl Into<String>) -> Result<T> {
    Err(Error::Protocol(msg.into()))
}
