//! Dependency-free readers for the data files shipped with Frontier: Elite II (DOS, 1993).
//!
//! Every parser cites the routine in `FRONTIER.EXE` (or an `EL2*.OVL` overlay) that reads the
//! same bytes, using the segment:offset address from the Ghidra export (see `docs/FINDINGS.md`).

pub mod mz;

/// Errors returned by the readers in this crate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// Input ended before a structure was complete.
    Truncated { needed: usize, available: usize },
    /// A magic number or invariant did not hold.
    Invalid(&'static str),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Truncated { needed, available } => {
                write!(f, "truncated: needed {needed} bytes, {available} available")
            }
            Error::Invalid(what) => write!(f, "invalid data: {what}"),
        }
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;
