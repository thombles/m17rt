use std::io::{Read, Write};

/// A TNC that supports reading and writing M17 KISS messages.
///
/// TNCs must be cloneable to support reading and writing from different threads,
/// via a working implementation of try_clone(). We do not require `Clone` directly
/// as this could not be fulfilled by `TcpStream`.
pub trait Tnc: Read + Write + Sized + Send + 'static {
    /// Return a copy of this TNC.
    ///
    /// `M17App` will use this to create a second instance of the supplied TNC then use
    /// one of them for reading and one of them for writing, concurrently across two threads.
    ///
    /// Implementations do not need to worry about trying to make two simultaneous reads or
    /// two simultaneous writes do something sensible. `M17App` will not do this and it would
    /// probably produce garbled KISS messages anyway.
    fn try_clone(&mut self) -> Result<Self, TncError>;

    /// Start I/O.
    fn start(&mut self) -> Result<(), TncError>;

    /// Shut down I/O - it is assumed we cannot restart.
    fn close(&mut self) -> Result<(), TncError>;
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum TncError {
    // TODO: Good error cases
    Unknown,
}

impl Tnc for std::net::TcpStream {
    fn try_clone(&mut self) -> Result<Self, TncError> {
        std::net::TcpStream::try_clone(self).map_err(|_| TncError::Unknown)
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
