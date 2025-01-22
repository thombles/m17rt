use thiserror::Error;

#[derive(Debug, Error)]
pub enum M17Error {
    #[error("given callsign contains at least one character invalid in M17: {0}")]
    InvalidCallsignCharacters(char),

    #[error("given callsign is {0} characters long; maximum is 9")]
    CallsignTooLong(usize),
}
