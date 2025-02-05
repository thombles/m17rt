use std::{fmt::Display, path::PathBuf};

use thiserror::Error;

/// Errors from the M17 Rust Toolkit
#[derive(Debug, Error)]
pub enum M17Error {
    #[error("given callsign contains at least one character invalid in M17: {0}")]
    InvalidCallsignCharacters(char),

    #[error("given callsign is {0} characters long; maximum is 9")]
    CallsignTooLong(usize),

    #[error(
        "provided packet payload is too large: provided {provided} bytes, capacity {capacity}"
    )]
    PacketTooLarge { provided: usize, capacity: usize },

    #[error("provided path to RRC file could not be opened: {0}")]
    InvalidRrcPath(PathBuf),

    #[error("failed to read from RRC file: {0}")]
    RrcReadFailed(PathBuf),

    #[error("tried to start app more than once")]
    InvalidStart,

    #[error("tried to close app that is not started")]
    InvalidClose,

    #[error("adapter error for id {0}: {1}")]
    Adapter(usize, #[source] AdapterError),

    #[error("soundmodem component error: {0}")]
    Soundmodem(#[source] SoundmodemError),
}

/// Arbitrary error type returned from adapters, which may be user-implemented
pub type AdapterError = Box<dyn std::error::Error + Sync + Send + 'static>;

/// Arbitrary error type returned from soundmodem components, which may be user-implemented
pub type SoundmodemError = Box<dyn std::error::Error + Sync + Send + 'static>;

/// Iterator over potentially multiple errors
#[derive(Debug, Error)]
pub struct M17Errors(pub(crate) Vec<M17Error>);
impl Iterator for M17Errors {
    type Item = M17Error;

    fn next(&mut self) -> Option<Self::Item> {
        self.0.pop()
    }
}

impl Display for M17Errors {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut displays = vec![];
        for e in &self.0 {
            displays.push(e.to_string());
        }
        write!(f, "[{}]", displays.join(", "))
    }
}

#[derive(Debug, Error)]
pub enum M17SoundmodemError {}
