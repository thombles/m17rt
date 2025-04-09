#![doc = include_str!("../README.md")]

use codec2::{Codec2, Codec2Mode};
use cpal::traits::DeviceTrait;
use cpal::traits::HostTrait;
use cpal::traits::StreamTrait;
use cpal::{Sample, SampleFormat, SampleRate};
use log::debug;
use m17app::adapter::StreamAdapter;
use m17app::app::TxHandle;
use m17app::error::AdapterError;
use m17app::link_setup::LinkSetup;
use m17app::link_setup::M17Address;
use m17app::StreamFrame;
use std::collections::VecDeque;
use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::sync::{
    mpsc::{channel, Receiver, Sender},
    Arc, Mutex,
};
use std::time::Duration;
use std::time::Instant;
use thiserror::Error;

pub fn decode_codec2<P: AsRef<Path>>(data: &[u8], out_path: P) -> Vec<i16> {
    let codec2 = Codec2::new(Codec2Mode::MODE_3200);
    let var_name = codec2;
    let mut codec = var_name;
    let mut all_samples: Vec<i16> = vec![];
    for i in 0..(data.len() / 8) {
        let mut samples = vec![0; codec.samples_per_frame()];
        codec.decode(&mut samples, &data[i * 8..((i + 1) * 8)]);
        all_samples.append(&mut samples);
    }

    // dude this works
    let mut speech_out = File::create(out_path).unwrap();
    for b in &all_samples {
        speech_out.write_all(&b.to_le_bytes()).unwrap();
    }
    all_samples
}

/// Subscribes to M17 streams and attempts to play the decoded Codec2
pub struct Codec2Adapter {
    state: Arc<Mutex<AdapterState>>,
    output_card: Option<String>,
}

impl Codec2Adapter {
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(AdapterState {
                tx: None,
                out_buf: VecDeque::new(),
                codec2: Codec2::new(Codec2Mode::MODE_3200),
                end_tx: None,
            })),
            // TODO: this doesn't work on rpi. Use default_output_device() by default
            output_card: None,
        }
    }

    pub fn set_output_card<S: Into<String>>(&mut self, card_name: S) {
        self.output_card = Some(card_name.into());
    }
}

impl Default for Codec2Adapter {
    fn default() -> Self {
        Self::new()
    }
}

struct AdapterState {
    tx: Option<TxHandle>,
    /// Circular buffer of output samples for playback
    out_buf: VecDeque<i16>,
    codec2: Codec2,
    end_tx: Option<Sender<()>>,
}

impl StreamAdapter for Codec2Adapter {
    fn start(&self, handle: TxHandle) -> Result<(), AdapterError> {
        self.state.lock().unwrap().tx = Some(handle);

        let (end_tx, end_rx) = channel();
        let (setup_tx, setup_rx) = channel();
        let state = self.state.clone();
        let output_card = self.output_card.clone();
        std::thread::spawn(move || stream_thread(end_rx, setup_tx, state, output_card));
        self.state.lock().unwrap().end_tx = Some(end_tx);
        // Propagate any errors arising in the thread
        setup_rx.recv()?
    }

    fn close(&self) -> Result<(), AdapterError> {
        let mut state = self.state.lock().unwrap();
        state.tx = None;
        state.end_tx = None;
        Ok(())
    }

    fn stream_began(&self, _link_setup: LinkSetup) {
        // for now we will assume:
        // - unencrypted
        // - data type is Voice (Codec2 3200), not Voice+Data
        // TODO: is encryption handled here or in M17App, such that we get a decrypted stream?
        // TODO: handle the Voice+Data combination with Codec2 1600
        self.state.lock().unwrap().codec2 = Codec2::new(Codec2Mode::MODE_3200);
    }

    fn stream_data(&self, _frame_number: u16, _is_final: bool, data: Arc<[u8; 16]>) {
        let mut state = self.state.lock().unwrap();
        for encoded in data.chunks(8) {
            if state.out_buf.len() < 1024 {
                let mut samples = [i16::EQUILIBRIUM; 160]; // while assuming 3200
                state.codec2.decode(&mut samples, encoded);
                // TODO: maybe get rid of VecDeque so we can decode directly into ring buffer?
                for s in samples {
                    state.out_buf.push_back(s);
                }
            } else {
                debug!("out_buf overflow");
            }
        }
    }
}

fn output_cb(data: &mut [i16], state: &Mutex<AdapterState>) {
    let mut state = state.lock().unwrap();
    for d in data {
        *d = state.out_buf.pop_front().unwrap_or(i16::EQUILIBRIUM);
    }
}

/// Create and manage the stream from a dedicated thread since it's `!Send`
fn stream_thread(
    end: Receiver<()>,
    setup_tx: Sender<Result<(), AdapterError>>,
    state: Arc<Mutex<AdapterState>>,
    output_card: Option<String>,
) {
    let host = cpal::default_host();
    let device = if let Some(output_card) = output_card {
        // TODO: more error handling for unwraps
        match host
            .output_devices()
            .unwrap()
            .find(|d| d.name().unwrap() == output_card)
        {
            Some(d) => d,
            None => {
                let _ = setup_tx.send(Err(M17Codec2Error::CardUnavailable(output_card).into()));
                return;
            }
        }
    } else {
        match host.default_output_device() {
            Some(d) => d,
            None => {
                let _ = setup_tx.send(Err(M17Codec2Error::DefaultCardUnavailable.into()));
                return;
            }
        }
    };
    let card_name = device.name().unwrap();
    let mut configs = match device.supported_output_configs() {
        Ok(c) => c,
        Err(e) => {
            let _ = setup_tx.send(Err(
                M17Codec2Error::OutputConfigsUnavailable(card_name, e).into()
            ));
            return;
        }
    };
    // TODO: channels == 1 doesn't work on a Raspberry Pi
    // make this configurable and support interleaving LRLR stereo samples if using 2 channels
    let config = match configs.find(|c| c.channels() == 1 && c.sample_format() == SampleFormat::I16)
    {
        Some(c) => c,
        None => {
            let _ = setup_tx.send(Err(
                M17Codec2Error::SupportedOutputUnavailable(card_name).into()
            ));
            return;
        }
    };

    let config = config.with_sample_rate(SampleRate(8000));
    let stream = match device.build_output_stream(
        &config.into(),
        move |data: &mut [i16], _info: &cpal::OutputCallbackInfo| {
            output_cb(data, &state);
        },
        |e| {
            // trigger end_tx here? always more edge cases
            debug!("error occurred in codec2 playback: {e:?}");
        },
        None,
    ) {
        Ok(s) => s,
        Err(e) => {
            let _ = setup_tx.send(Err(
                M17Codec2Error::OutputStreamBuildError(card_name, e).into()
            ));
            return;
        }
    };
    match stream.play() {
        Ok(()) => (),
        Err(e) => {
            let _ = setup_tx.send(Err(
                M17Codec2Error::OutputStreamPlayError(card_name, e).into()
            ));
            return;
        }
    }
    let _ = setup_tx.send(Ok(()));
    let _ = end.recv();
    // it seems concrete impls of Stream have a Drop implementation that will handle termination
}

/// Transmits a wave file as an M17 stream
pub struct WavePlayer;

impl WavePlayer {
    /// Plays a wave file (blocking).
    ///
    /// * `path`: wave file to transmit, must be 8 kHz mono and 16-bit LE
    /// * `tx`: a `TxHandle` obtained from an `M17App`
    /// * `source`: address of transmission source
    /// * `destination`: address of transmission destination
    /// * `channel_access_number`: from 0 to 15, usually 0
    pub fn play(
        path: PathBuf,
        tx: TxHandle,
        source: &M17Address,
        destination: &M17Address,
        channel_access_number: u8,
    ) {
        let mut reader = hound::WavReader::open(path).unwrap();
        let mut samples = reader.samples::<i16>();

        let mut codec = Codec2::new(Codec2Mode::MODE_3200);
        let mut in_buf = [0i16; 160];
        let mut out_buf = [0u8; 16];
        let mut lsf_chunk: usize = 0;
        const TICK: Duration = Duration::from_millis(40);
        let mut next_tick = Instant::now() + TICK;
        let mut frame_number = 0;

        let mut setup = LinkSetup::new_voice(source, destination);
        setup.set_channel_access_number(channel_access_number);
        tx.transmit_stream_start(&setup);

        loop {
            let mut last_one = false;
            for out in out_buf.chunks_mut(8) {
                for i in in_buf.iter_mut() {
                    let sample = match samples.next() {
                        Some(Ok(sample)) => sample,
                        _ => {
                            last_one = true;
                            0
                        }
                    };
                    *i = sample;
                }
                codec.encode(out, &in_buf);
            }
            tx.transmit_stream_next(&StreamFrame {
                lich_idx: lsf_chunk as u8,
                lich_part: setup.lich_part(lsf_chunk as u8),
                frame_number,
                end_of_stream: last_one,
                stream_data: out_buf,
            });
            frame_number += 1;
            lsf_chunk = (lsf_chunk + 1) % 6;

            if last_one {
                break;
            }

            std::thread::sleep(next_tick.duration_since(Instant::now()));
            next_tick += TICK;
        }
    }
}

#[derive(Debug, Error)]
pub enum M17Codec2Error {
    #[error("selected card '{0}' does not exist or is in use")]
    CardUnavailable(String),

    #[error("default output card is unavailable")]
    DefaultCardUnavailable,

    #[error("selected card '{0}' failed to list available output configs: '{1}'")]
    OutputConfigsUnavailable(String, #[source] cpal::SupportedStreamConfigsError),

    #[error("selected card '{0}' did not offer a compatible output config type, either due to hardware limitations or because it is currently in use")]
    SupportedOutputUnavailable(String),

    #[error("selected card '{0}' was unable to build an output stream: '{1}'")]
    OutputStreamBuildError(String, #[source] cpal::BuildStreamError),

    #[error("selected card '{0}' was unable to play an output stream: '{1}'")]
    OutputStreamPlayError(String, #[source] cpal::PlayStreamError),
}
