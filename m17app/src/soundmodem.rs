use std::io::{self, ErrorKind, Read, Write};

use crate::tnc::{Tnc, TncError};
use log::debug;
use m17core::tnc::SoftTnc;
use std::fs::File;
use std::path::PathBuf;
use std::sync::mpsc::{channel, sync_channel, Receiver, Sender, SyncSender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub struct Soundmodem {
    tnc: SoftTnc,
    config: SoundmodemConfig,
}

pub struct SoundmodemConfig {
    // sound cards, PTT, etc.
    input: Box<dyn InputSource>,
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

pub enum SoundmodemEvent {
    Kiss(Arc<[u8]>),
    BasebandInput(Arc<[i16]>),
}

pub trait InputSource: Send + Sync + 'static {
    fn start(&self, samples: SyncSender<SoundmodemEvent>);
    fn close(&self);
}

pub struct InputSoundcard {
    cpal_name: String,
}

impl InputSource for InputSoundcard {
    fn start(&self, samples: SyncSender<SoundmodemEvent>) {
        todo!()
    }

    fn close(&self) {
        todo!()
    }
}

pub struct InputRrcFile {
    path: PathBuf,
    end_tx: Mutex<Option<Sender<()>>>,
}

impl InputSource for InputRrcFile {
    fn start(&self, samples: SyncSender<SoundmodemEvent>) {
        let (end_tx, end_rx) = channel();
        let path = self.path.clone();
        std::thread::spawn(move || {
            // TODO: error handling
            let mut file = File::open(path).unwrap();
            let mut baseband = vec![];
            file.read_to_end(&mut baseband).unwrap();

            // assuming 48 kHz for now
            const TICK: Duration = Duration::from_millis(25);
            const SAMPLES_PER_TICK: usize = 1200;

            let mut next_tick = Instant::now() + TICK;
            let mut buf = [0i16; SAMPLES_PER_TICK];
            let mut idx = 0;

            for sample in baseband
                .chunks(2)
                .map(|pair| i16::from_le_bytes([pair[0], pair[1]]))
            {
                buf[idx] = sample;
                idx += 1;
                if idx == SAMPLES_PER_TICK {
                    if let Err(e) = samples.try_send(SoundmodemEvent::BasebandInput(buf.into())) {
                        debug!("overflow feeding soundmodem: {e:?}");
                    }
                    next_tick = next_tick + TICK;
                    idx = 0;
                    std::thread::sleep(next_tick.duration_since(Instant::now()));
                }
                if end_rx.try_recv() != Err(TryRecvError::Empty) {
                    break;
                }
            }
        });
        *self.end_tx.lock().unwrap() = Some(end_tx);
    }

    fn close(&self) {
        let _ = self.end_tx.lock().unwrap().take();
    }
}
