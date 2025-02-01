use std::io::{Read, Write};

use crate::tnc::Tnc;

#[derive(Clone)]
pub(crate) struct NullTnc;

impl Tnc for NullTnc {
    fn try_clone(&mut self) -> Result<Self, crate::tnc::TncError> {
        Ok(self.clone())
    }

    fn start(&mut self) -> Result<(), crate::tnc::TncError> {
        Ok(())
    }

    fn close(&mut self) -> Result<(), crate::tnc::TncError> {
        Ok(())
    }
}

impl Write for NullTnc {
    fn write(&mut self, _buf: &[u8]) -> std::io::Result<usize> {
        Ok(0)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Read for NullTnc {
    fn read(&mut self, _buf: &mut [u8]) -> std::io::Result<usize> {
        Ok(0)
    }
}
