use std::io::{self, ErrorKind, Read, Write};

use m17core::tnc::SoftTnc;

///
pub trait Tnc: Read + Write + Sized {
    fn try_clone(&mut self) -> Result<Self, TncError>;
    fn start(&mut self) -> Result<(), TncError>;
    fn close(&mut self) -> Result<(), TncError>;
}

#[derive(Debug)]
pub enum TncError {
    General(String),
}

// TODO: move the following to its own module

pub struct Soundmodem {
    tnc: SoftTnc,
    config: SoundmodemConfig,
}

pub struct SoundmodemConfig {
    // sound cards, PTT, etc.
}

impl Read for Soundmodem {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.tnc
            .read_kiss(buf)
            .map_err(|s| io::Error::new(ErrorKind::Other, format!("{:?}", s)))
    }
}

impl Write for Soundmodem {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.tnc
            .write_kiss(buf)
            .map_err(|s| io::Error::new(ErrorKind::Other, format!("{:?}", s)))
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Tnc for Soundmodem {
    fn try_clone(&mut self) -> Result<Self, TncError> {
        unimplemented!();
    }

    fn start(&mut self) -> Result<(), TncError> {
        unimplemented!();
    }

    fn close(&mut self) -> Result<(), TncError> {
        unimplemented!();
    }
}
