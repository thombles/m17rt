use std::io::{Read, Write};

/// A TNC that supports reading and writing M17 KISS messages.
///
/// TNCs must be cloneable to support reading and writing from different threads,
/// via a working implementation of try_clone(). We do not require `Clone` directly
/// as this could not be fulfilled by `TcpStream`.
pub trait Tnc: Read + Write + Sized + Send + 'static {
    fn try_clone(&mut self) -> Result<Self, TncError>;
    fn start(&mut self) -> Result<(), TncError>;
    fn close(&mut self) -> Result<(), TncError>;
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum TncError {
    // TODO: Good error cases
    Unknown,
}

impl Tnc for std::net::TcpStream {
    fn try_clone(&mut self) -> Result<Self, TncError> {
        self.try_clone().map_err(|_| TncError::Unknown)
    }

    fn start(&mut self) -> Result<(), TncError> {
        // already started, hopefully we get onto reading the socket quickly
        Ok(())
    }

    fn close(&mut self) -> Result<(), TncError> {
        self.shutdown(std::net::Shutdown::Both)
            .map_err(|_| TncError::Unknown)
    }
}
