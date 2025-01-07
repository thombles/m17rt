use std::io::{self, ErrorKind, Read, Write};

use crate::tnc::{Tnc, TncError};
use cpal::traits::DeviceTrait;
use cpal::traits::HostTrait;
use cpal::traits::StreamTrait;
use cpal::{SampleFormat, SampleRate};
use log::debug;
use m17core::kiss::MAX_FRAME_LEN;
use m17core::modem::{Demodulator, SoftDemodulator};
use m17core::tnc::SoftTnc;
use std::fs::File;
use std::path::PathBuf;
use std::sync::mpsc::{channel, sync_channel, Receiver, Sender, SyncSender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub struct Soundmodem {
    event_tx: SyncSender<SoundmodemEvent>,
    kiss_out_rx: Arc<Mutex<Receiver<Arc<[u8]>>>>,
    partial_kiss_out: Arc<Mutex<Option<PartialKissOut>>>,
}

impl Soundmodem {
    pub fn new_with_input<T: InputSource>(input: T) -> Self {
        // must create TNC here
        let (event_tx, event_rx) = sync_channel(128);
        let (kiss_out_tx, kiss_out_rx) = sync_channel(128);
        spawn_soundmodem_worker(event_tx.clone(), event_rx, kiss_out_tx, Box::new(input));
        Self {
            event_tx,
            kiss_out_rx: Arc::new(Mutex::new(kiss_out_rx)),
            partial_kiss_out: Arc::new(Mutex::new(None)),
        }
    }
}

struct PartialKissOut {
    output: Arc<[u8]>,
    idx: usize,
}

impl Read for Soundmodem {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        {
            let mut partial_kiss_out = self.partial_kiss_out.lock().unwrap();
            if let Some(partial) = partial_kiss_out.as_mut() {
                let remaining = partial.output.len() - partial.idx;
                let to_write = remaining.min(buf.len());
                buf[0..to_write]
                    .copy_from_slice(&partial.output[partial.idx..(partial.idx + to_write)]);
                if to_write == remaining {
                    *partial_kiss_out = None;
                } else {
                    partial.idx += to_write;
                }
                return Ok(to_write);
            }
        }
        let output = {
            let rx = self.kiss_out_rx.lock().unwrap();
            rx.recv()
                .map_err(|s| io::Error::new(ErrorKind::Other, format!("{:?}", s)))?
        };
        let to_write = output.len().min(buf.len());
        buf[0..to_write].copy_from_slice(&output[0..to_write]);
        if to_write != output.len() {
            *self.partial_kiss_out.lock().unwrap() = Some(PartialKissOut {
                output,
                idx: to_write,
            })
        }
        Ok(to_write)
    }
}

impl Write for Soundmodem {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let _ = self.event_tx.try_send(SoundmodemEvent::Kiss(buf.into()));
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Tnc for Soundmodem {
    fn try_clone(&mut self) -> Result<Self, TncError> {
        Ok(Self {
            event_tx: self.event_tx.clone(),
            kiss_out_rx: self.kiss_out_rx.clone(),
            partial_kiss_out: self.partial_kiss_out.clone(),
        })
    }

    fn start(&mut self) -> Result<(), TncError> {
        let _ = self.event_tx.send(SoundmodemEvent::Start);
        Ok(())
    }

    fn close(&mut self) -> Result<(), TncError> {
        let _ = self.event_tx.send(SoundmodemEvent::Close);
        Ok(())
    }
}

pub enum SoundmodemEvent {
    Kiss(Arc<[u8]>),
    BasebandInput(Arc<[i16]>),
    Start,
    Close,
}

fn spawn_soundmodem_worker(
    event_tx: SyncSender<SoundmodemEvent>,
    event_rx: Receiver<SoundmodemEvent>,
    kiss_out_tx: SyncSender<Arc<[u8]>>,
    input: Box<dyn InputSource>,
) {
    std::thread::spawn(move || {
        // TODO: should be able to provide a custom Demodulator for a soundmodem
        let mut demod = SoftDemodulator::new();
        let mut tnc = SoftTnc::new();
        let mut buf = [0u8; MAX_FRAME_LEN];
        while let Ok(ev) = event_rx.recv() {
            match ev {
                SoundmodemEvent::Kiss(k) => {
                    let _n = tnc.write_kiss(&k);
                    // TODO: what does it mean if we fail to write it all?
                    // Probably we have to read frames for tx first - revisit this during tx
                }
                SoundmodemEvent::BasebandInput(b) => {
                    for sample in &*b {
                        if let Some(frame) = demod.demod(*sample) {
                            tnc.handle_frame(frame);
                            loop {
                                let n = tnc.read_kiss(&mut buf);
                                if n > 0 {
                                    let _ = kiss_out_tx.try_send(buf[0..n].into());
                                } else {
                                    break;
                                }
                            }
                        }
                    }
                }
                SoundmodemEvent::Start => input.start(event_tx.clone()),
                SoundmodemEvent::Close => break,
            }
        }
    });
}

pub trait InputSource: Send + Sync + 'static {
    fn start(&self, samples: SyncSender<SoundmodemEvent>);
    fn close(&self);
}

pub struct InputSoundcard {
    cpal_name: Option<String>,
    end_tx: Mutex<Option<Sender<()>>>,
}

impl InputSoundcard {
    pub fn new() -> Self {
        Self {
            cpal_name: None,
            end_tx: Mutex::new(None),
        }
    }

    pub fn new_with_card(card_name: String) -> Self {
        Self {
            cpal_name: Some(card_name),
            end_tx: Mutex::new(None),
        }
    }
}

impl InputSource for InputSoundcard {
    fn start(&self, samples: SyncSender<SoundmodemEvent>) {
        let (end_tx, end_rx) = channel();
        let cpal_name = self.cpal_name.clone();
        std::thread::spawn(move || {
            let host = cpal::default_host();
            let device = if let Some(name) = cpal_name.as_deref() {
                host.input_devices()
                    .unwrap()
                    .find(|d| d.name().unwrap() == name)
                    .unwrap()
            } else {
                host.default_input_device().unwrap()
            };
            let mut configs = device.supported_input_configs().unwrap();
            let config = configs
                .find(|c| c.channels() == 1 && c.sample_format() == SampleFormat::I16)
                .unwrap()
                .with_sample_rate(SampleRate(48000));
            let stream = device
                .build_input_stream(
                    &config.into(),
                    move |data: &[i16], _info: &cpal::InputCallbackInfo| {
                        debug!("input has given us {} samples", data.len());
                        let out: Vec<i16> = data.iter().map(|s| *s).collect();
                        let _ = samples.try_send(SoundmodemEvent::BasebandInput(out.into()));
                    },
                    |e| {
                        // TODO: abort?
                        debug!("error occurred in soundcard input: {e:?}");
                    },
                    None,
                )
                .unwrap();
            stream.play().unwrap();
            let _ = end_rx.recv();
        });
        *self.end_tx.lock().unwrap() = Some(end_tx);
    }

    fn close(&self) {
        let _ = self.end_tx.lock().unwrap().take();
    }
}

pub struct InputRrcFile {
    path: PathBuf,
    end_tx: Mutex<Option<Sender<()>>>,
}

impl InputRrcFile {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            end_tx: Mutex::new(None),
        }
    }
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
