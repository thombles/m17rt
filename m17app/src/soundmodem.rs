use crate::error::{M17Error, SoundmodemError};
use crate::tnc::{Tnc, TncError};
use m17core::kiss::MAX_FRAME_LEN;
use m17core::modem::{Demodulator, Modulator, ModulatorAction, SoftDemodulator, SoftModulator};
use m17core::tnc::SoftTnc;
use std::collections::VecDeque;
use std::fs::File;
use std::io::{self, ErrorKind, Read, Write};
use std::path::PathBuf;
use std::sync::mpsc::{channel, sync_channel, Receiver, Sender, SyncSender, TryRecvError};
use std::sync::RwLock;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use thiserror::Error;

pub struct Soundmodem {
    event_tx: SyncSender<SoundmodemEvent>,
    kiss_out_rx: Arc<Mutex<Receiver<Arc<[u8]>>>>,
    partial_kiss_out: Arc<Mutex<Option<PartialKissOut>>>,
    error_handler: ErrorHandlerInternal,
}

impl Soundmodem {
    pub fn new<I: InputSource, O: OutputSink, P: Ptt, E: ErrorHandler>(
        input: I,
        output: O,
        ptt: P,
        error: E,
    ) -> Self {
        let (event_tx, event_rx) = sync_channel(128);
        let (kiss_out_tx, kiss_out_rx) = sync_channel(128);
        let runtime_error_handler: ErrorHandlerInternal = Arc::new(Mutex::new(Box::new(error)));
        spawn_soundmodem_worker(
            event_tx.clone(),
            event_rx,
            kiss_out_tx,
            Box::new(input),
            Box::new(output),
            Box::new(ptt),
            runtime_error_handler.clone(),
        );
        Self {
            event_tx,
            kiss_out_rx: Arc::new(Mutex::new(kiss_out_rx)),
            partial_kiss_out: Arc::new(Mutex::new(None)),
            error_handler: runtime_error_handler,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum ErrorSource {
    Input,
    Output,
    Ptt,
}

pub trait ErrorHandler: Send + Sync + 'static {
    fn soundmodem_error(&mut self, source: ErrorSource, err: SoundmodemError);
}

impl<F> ErrorHandler for F
where
    F: FnMut(ErrorSource, SoundmodemError) + Send + Sync + 'static,
{
    fn soundmodem_error(&mut self, source: ErrorSource, err: SoundmodemError) {
        self(source, err)
    }
}

pub struct NullErrorHandler;

impl NullErrorHandler {
    pub fn new() -> Self {
        Self {}
    }
}

impl Default for NullErrorHandler {
    fn default() -> Self {
        Self::new()
    }
}

impl ErrorHandler for NullErrorHandler {
    fn soundmodem_error(&mut self, source: ErrorSource, err: SoundmodemError) {
        let _ = source;
        let _ = err;
    }
}

type ErrorHandlerInternal = Arc<Mutex<Box<dyn ErrorHandler>>>;

#[derive(Clone)]
pub struct SoundmodemErrorSender {
    source: ErrorSource,
    event_tx: SyncSender<SoundmodemEvent>,
}

impl SoundmodemErrorSender {
    pub fn send_error<E: Into<SoundmodemError>>(&self, err: E) {
        let _ = self
            .event_tx
            .send(SoundmodemEvent::RuntimeError(self.source, err.into()));
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
            error_handler: self.error_handler.clone(),
        })
    }

    fn start(&mut self) {
        let _ = self.event_tx.send(SoundmodemEvent::Start);
    }

    fn close(&mut self) {
        let _ = self.event_tx.send(SoundmodemEvent::Close);
    }
}

pub enum SoundmodemEvent {
    Kiss(Arc<[u8]>),
    BasebandInput(Arc<[i16]>),
    Start,
    Close,
    DidReadFromOutputBuffer { len: usize, timestamp: Instant },
    OutputUnderrun,
    RuntimeError(ErrorSource, SoundmodemError),
}

fn spawn_soundmodem_worker(
    event_tx: SyncSender<SoundmodemEvent>,
    event_rx: Receiver<SoundmodemEvent>,
    kiss_out_tx: SyncSender<Arc<[u8]>>,
    input: Box<dyn InputSource>,
    output: Box<dyn OutputSink>,
    mut ptt_driver: Box<dyn Ptt>,
    error_handler: ErrorHandlerInternal,
) {
    std::thread::spawn(move || {
        // TODO: should be able to provide a custom Demodulator for a soundmodem
        let mut demodulator = SoftDemodulator::new();
        let mut modulator = SoftModulator::new();
        let mut tnc = SoftTnc::new();
        let mut buf = [0u8; MAX_FRAME_LEN];
        let out_buffer = Arc::new(RwLock::new(OutputBuffer::new()));
        let mut out_samples = [0i16; 1024];
        let start = Instant::now();
        let mut ptt = false;
        while let Ok(ev) = event_rx.recv() {
            // Update clock on TNC before we do anything
            let sample_time = start.elapsed();
            let secs = sample_time.as_secs();
            let nanos = sample_time.subsec_nanos();
            // Accurate to within approx 1 sample
            let now_samples = 48000 * secs + (nanos as u64 / 20833);
            tnc.set_now(now_samples);

            // Handle event
            match ev {
                SoundmodemEvent::Kiss(k) => {
                    let _n = tnc.write_kiss(&k);
                    // TODO: what does it mean if we fail to write it all?
                    // Probably we have to read frames for tx first - revisit this during tx
                }
                SoundmodemEvent::BasebandInput(b) => {
                    for sample in &*b {
                        if let Some(frame) = demodulator.demod(*sample) {
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
                    tnc.set_data_carrier_detect(demodulator.data_carrier_detect());
                }
                SoundmodemEvent::Start => {
                    let input_errors = SoundmodemErrorSender {
                        source: ErrorSource::Input,
                        event_tx: event_tx.clone(),
                    };
                    input.start(event_tx.clone(), input_errors);
                    let output_errors = SoundmodemErrorSender {
                        source: ErrorSource::Output,
                        event_tx: event_tx.clone(),
                    };
                    output.start(event_tx.clone(), out_buffer.clone(), output_errors);
                }
                SoundmodemEvent::Close => {
                    input.close();
                    output.close();
                    if let Err(e) = ptt_driver.ptt_off() {
                        error_handler
                            .lock()
                            .unwrap()
                            .soundmodem_error(ErrorSource::Ptt, e);
                    }
                    break;
                }
                SoundmodemEvent::DidReadFromOutputBuffer { len, timestamp } => {
                    let (occupied, internal_latency) = {
                        let out_buffer = out_buffer.read().unwrap();
                        (out_buffer.samples.len(), out_buffer.latency)
                    };
                    let internal_latency = (internal_latency.as_secs_f32() * 48000.0) as usize;
                    let dynamic_latency =
                        len.saturating_sub((timestamp.elapsed().as_secs_f32() * 48000.0) as usize);
                    modulator.update_output_buffer(
                        occupied,
                        48000,
                        internal_latency + dynamic_latency,
                    );
                }
                SoundmodemEvent::OutputUnderrun => {
                    log::debug!("output underrun");
                    // TODO: cancel transmission, send empty data frame to host
                }
                SoundmodemEvent::RuntimeError(source, err) => {
                    error_handler.lock().unwrap().soundmodem_error(source, err);
                }
            }

            // Update PTT state
            let new_ptt = tnc.ptt();
            if new_ptt != ptt {
                if new_ptt {
                    if let Err(e) = ptt_driver.ptt_on() {
                        error_handler
                            .lock()
                            .unwrap()
                            .soundmodem_error(ErrorSource::Ptt, e);
                    }
                } else if let Err(e) = ptt_driver.ptt_off() {
                    error_handler
                        .lock()
                        .unwrap()
                        .soundmodem_error(ErrorSource::Ptt, e);
                }
            }
            ptt = new_ptt;

            // Let the modulator do what it wants
            while let Some(action) = modulator.run() {
                match action {
                    ModulatorAction::SetIdle(idling) => {
                        out_buffer.write().unwrap().idling = idling;
                    }
                    ModulatorAction::GetNextFrame => {
                        modulator.provide_next_frame(tnc.read_tx_frame());
                    }
                    ModulatorAction::ReadOutput => loop {
                        let n = modulator.read_output_samples(&mut out_samples);
                        if n == 0 {
                            break;
                        }
                        let mut out_buffer = out_buffer.write().unwrap();
                        for s in &out_samples[0..n] {
                            out_buffer.samples.push_back(*s);
                        }
                    },
                    ModulatorAction::TransmissionWillEnd(in_samples) => {
                        tnc.set_tx_end_time(in_samples);
                    }
                }
            }
        }
    });
}

pub trait InputSource: Send + Sync + 'static {
    fn start(&self, samples: SyncSender<SoundmodemEvent>, errors: SoundmodemErrorSender);
    fn close(&self);
}

pub struct InputRrcFile {
    baseband: Arc<[u8]>,
    end_tx: Mutex<Option<Sender<()>>>,
}

impl InputRrcFile {
    pub fn new(path: PathBuf) -> Result<Self, M17Error> {
        let mut file = File::open(&path).map_err(|_| M17Error::InvalidRrcPath(path.clone()))?;
        let mut baseband = vec![];
        file.read_to_end(&mut baseband)
            .map_err(|_| M17Error::RrcReadFailed(path))?;
        Ok(Self {
            baseband: baseband.into(),
            end_tx: Mutex::new(None),
        })
    }
}

impl InputSource for InputRrcFile {
    fn start(&self, samples: SyncSender<SoundmodemEvent>, errors: SoundmodemErrorSender) {
        let (end_tx, end_rx) = channel();
        let baseband = self.baseband.clone();
        std::thread::spawn(move || {
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
                    if samples
                        .try_send(SoundmodemEvent::BasebandInput(buf.into()))
                        .is_err()
                    {
                        errors.send_error(InputRrcError::Overflow);
                    }
                    next_tick += TICK;
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

#[derive(Debug, Error)]
pub enum InputRrcError {
    #[error("overflow occurred feeding sample to soundmodem")]
    Overflow,
}

pub struct NullInputSource {
    end_tx: Mutex<Option<Sender<()>>>,
}

impl NullInputSource {
    pub fn new() -> Self {
        Self {
            end_tx: Mutex::new(None),
        }
    }
}

impl InputSource for NullInputSource {
    fn start(&self, samples: SyncSender<SoundmodemEvent>, errors: SoundmodemErrorSender) {
        let (end_tx, end_rx) = channel();
        std::thread::spawn(move || {
            // assuming 48 kHz for now
            const TICK: Duration = Duration::from_millis(25);
            const SAMPLES_PER_TICK: usize = 1200;
            let mut next_tick = Instant::now() + TICK;

            loop {
                std::thread::sleep(next_tick.duration_since(Instant::now()));
                next_tick += TICK;
                if end_rx.try_recv() != Err(TryRecvError::Empty) {
                    break;
                }
                if samples
                    .try_send(SoundmodemEvent::BasebandInput(
                        [0i16; SAMPLES_PER_TICK].into(),
                    ))
                    .is_err()
                {
                    errors.send_error(NullInputError::Overflow);
                }
            }
        });
        *self.end_tx.lock().unwrap() = Some(end_tx);
    }

    fn close(&self) {
        let _ = self.end_tx.lock().unwrap().take();
    }
}

#[derive(Debug, Error)]
pub enum NullInputError {
    #[error("overflow occurred feeding sample to soundmodem")]
    Overflow,
}

impl Default for NullInputSource {
    fn default() -> Self {
        Self::new()
    }
}

pub struct OutputBuffer {
    pub idling: bool,
    // TODO: something more efficient
    pub samples: VecDeque<i16>,
    pub latency: Duration,
}

impl OutputBuffer {
    pub fn new() -> Self {
        Self {
            idling: true,
            samples: VecDeque::new(),
            latency: Duration::ZERO,
        }
    }
}

impl Default for OutputBuffer {
    fn default() -> Self {
        Self::new()
    }
}

pub trait OutputSink: Send + Sync + 'static {
    fn start(
        &self,
        event_tx: SyncSender<SoundmodemEvent>,
        buffer: Arc<RwLock<OutputBuffer>>,
        errors: SoundmodemErrorSender,
    );
    fn close(&self);
}

pub struct OutputRrcFile {
    path: PathBuf,
    end_tx: Mutex<Option<Sender<()>>>,
}

impl OutputRrcFile {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            end_tx: Mutex::new(None),
        }
    }
}

impl OutputSink for OutputRrcFile {
    fn start(
        &self,
        event_tx: SyncSender<SoundmodemEvent>,
        buffer: Arc<RwLock<OutputBuffer>>,
        errors: SoundmodemErrorSender,
    ) {
        let (end_tx, end_rx) = channel();
        let mut file = match File::create(self.path.clone()) {
            Ok(f) => f,
            Err(e) => {
                errors.send_error(OutputRrcError::Open(e));
                return;
            }
        };
        std::thread::spawn(move || {
            // assuming 48 kHz for now
            const TICK: Duration = Duration::from_millis(25);
            const SAMPLES_PER_TICK: usize = 1200;

            // flattened BE i16s for writing
            let mut buf = [0u8; SAMPLES_PER_TICK * 2];
            let mut next_tick = Instant::now() + TICK;

            loop {
                std::thread::sleep(next_tick.duration_since(Instant::now()));
                next_tick += TICK;
                if end_rx.try_recv() != Err(TryRecvError::Empty) {
                    break;
                }
                // For now only write deliberately modulated (non-idling) samples
                // Multiple transmissions will get smooshed together
                let mut buf_used = 0;

                let mut buffer = buffer.write().unwrap();
                for out in buf.chunks_mut(2) {
                    if let Some(s) = buffer.samples.pop_front() {
                        let be = s.to_le_bytes();
                        out.copy_from_slice(&[be[0], be[1]]);
                        buf_used += 2;
                    } else if !buffer.idling {
                        let _ = event_tx.send(SoundmodemEvent::OutputUnderrun);
                        break;
                    }
                }
                if let Err(e) = file.write_all(&buf[0..buf_used]) {
                    errors.send_error(OutputRrcError::WriteError(e));
                    break;
                }
                let _ = event_tx.send(SoundmodemEvent::DidReadFromOutputBuffer {
                    len: buf_used / 2,
                    timestamp: Instant::now(),
                });
            }
        });
        *self.end_tx.lock().unwrap() = Some(end_tx);
    }

    fn close(&self) {
        let _ = self.end_tx.lock().unwrap().take();
    }
}

#[derive(Debug, Error)]
pub enum OutputRrcError {
    #[error("unable to open rrc file for writing: {0}")]
    Open(#[source] std::io::Error),

    #[error("error writing to output file: {0}")]
    WriteError(#[source] std::io::Error),
}

pub struct NullOutputSink {
    end_tx: Mutex<Option<Sender<()>>>,
}

impl NullOutputSink {
    pub fn new() -> Self {
        Self {
            end_tx: Mutex::new(None),
        }
    }
}

impl Default for NullOutputSink {
    fn default() -> Self {
        Self::new()
    }
}

impl OutputSink for NullOutputSink {
    fn start(
        &self,
        event_tx: SyncSender<SoundmodemEvent>,
        buffer: Arc<RwLock<OutputBuffer>>,
        _errors: SoundmodemErrorSender,
    ) {
        let (end_tx, end_rx) = channel();
        std::thread::spawn(move || {
            // assuming 48 kHz for now
            const TICK: Duration = Duration::from_millis(25);
            const SAMPLES_PER_TICK: usize = 1200;
            let mut next_tick = Instant::now() + TICK;

            loop {
                std::thread::sleep(next_tick.duration_since(Instant::now()));
                next_tick += TICK;
                if end_rx.try_recv() != Err(TryRecvError::Empty) {
                    break;
                }

                let mut buffer = buffer.write().unwrap();
                let mut taken = 0;
                for _ in 0..SAMPLES_PER_TICK {
                    if buffer.samples.pop_front().is_none() {
                        if !buffer.idling {
                            let _ = event_tx.send(SoundmodemEvent::OutputUnderrun);
                            break;
                        }
                    } else {
                        taken += 1;
                    }
                }
                let _ = event_tx.send(SoundmodemEvent::DidReadFromOutputBuffer {
                    len: taken,
                    timestamp: Instant::now(),
                });
            }
        });
        *self.end_tx.lock().unwrap() = Some(end_tx);
    }

    fn close(&self) {
        let _ = self.end_tx.lock().unwrap().take();
    }
}

pub trait Ptt: Send + 'static {
    fn ptt_on(&mut self) -> Result<(), SoundmodemError>;
    fn ptt_off(&mut self) -> Result<(), SoundmodemError>;
}

/// There is no PTT because this TNC will never make transmissions on a real radio.
pub struct NullPtt;

impl NullPtt {
    pub fn new() -> Self {
        Self
    }
}

impl Default for NullPtt {
    fn default() -> Self {
        Self::new()
    }
}

impl Ptt for NullPtt {
    fn ptt_on(&mut self) -> Result<(), SoundmodemError> {
        Ok(())
    }

    fn ptt_off(&mut self) -> Result<(), SoundmodemError> {
        Ok(())
    }
}
