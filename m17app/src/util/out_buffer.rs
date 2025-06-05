//! Buffer between `read()` calls

use std::{
    io::{self, ErrorKind, Read},
    sync::{Arc, Mutex, mpsc::Receiver},
};

#[derive(Clone)]
struct PartialOut {
    output: Arc<[u8]>,
    idx: usize,
}

/// Buffer binary chunks from an MPSC receiver, feeding arbitrary chunks to `read()` calls.
///
/// Can be cloned, but should only be read from once at a time.
#[derive(Clone)]
pub struct OutBuffer {
    rx: Arc<Mutex<Receiver<Arc<[u8]>>>>,
    partial_out: Arc<Mutex<Option<PartialOut>>>,
}

impl OutBuffer {
    pub fn new(rx: Receiver<Arc<[u8]>>) -> Self {
        Self {
            rx: Arc::new(Mutex::new(rx)),
            partial_out: Arc::new(Mutex::new(None)),
        }
    }
}

impl Read for OutBuffer {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        {
            let mut partial_out = self.partial_out.lock().unwrap();
            if let Some(partial) = partial_out.as_mut() {
                let remaining = partial.output.len() - partial.idx;
                let to_write = remaining.min(buf.len());
                buf[0..to_write]
                    .copy_from_slice(&partial.output[partial.idx..(partial.idx + to_write)]);
                if to_write == remaining {
                    *partial_out = None;
                } else {
                    partial.idx += to_write;
                }
                return Ok(to_write);
            }
        }
        let output = {
            let rx = self.rx.lock().unwrap();
            rx.recv()
                .map_err(|s| io::Error::new(ErrorKind::Other, format!("{:?}", s)))?
        };
        let to_write = output.len().min(buf.len());
        buf[0..to_write].copy_from_slice(&output[0..to_write]);
        if to_write != output.len() {
            *self.partial_out.lock().unwrap() = Some(PartialOut {
                output,
                idx: to_write,
            })
        }
        Ok(to_write)
    }
}
