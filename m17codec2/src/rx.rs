use crate::M17Codec2Error;
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
use rubato::Resampler;
use rubato::SincFixedIn;
use rubato::SincInterpolationParameters;
use std::collections::VecDeque;
use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::sync::{
    Arc, Mutex,
    mpsc::{Receiver, Sender, channel},
};

/// Write one or more 8-byte chunks of 3200-bit Codec2 to a raw S16LE file
/// and return the samples.
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
    let mut speech_out = File::create(out_path).unwrap();
    for b in &all_samples {
        speech_out.write_all(&b.to_le_bytes()).unwrap();
    }
    all_samples
}

/// Subscribes to M17 streams and attempts to play the decoded Codec2
pub struct Codec2RxAdapter {
    state: Arc<Mutex<AdapterState>>,
    output_card: Option<String>,
}

impl Codec2RxAdapter {
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(AdapterState {
                out_buf: VecDeque::new(),
                codec2: Codec2::new(Codec2Mode::MODE_3200),
                end_tx: None,
                resampler: None,
            })),
            output_card: None,
        }
    }

    pub fn set_output_card<S: Into<String>>(&mut self, card_name: S) {
        self.output_card = Some(card_name.into());
    }

    /// List sound cards supported for audio output.
    ///
    /// M17RT will handle any card with 1 or 2 channels and 16-bit output.
    pub fn supported_output_cards() -> Vec<String> {
        let mut out = vec![];
        let host = cpal::default_host();
        let Ok(output_devices) = host.output_devices() else {
            return out;
        };
        for d in output_devices {
            let Ok(mut configs) = d.supported_output_configs() else {
                continue;
            };
            if configs.any(|config| {
                (config.channels() == 1 || config.channels() == 2)
                    && config.sample_format() == SampleFormat::I16
            }) {
                let Ok(name) = d.name() else {
                    continue;
                };
                out.push(name);
            }
        }
        out.sort();
        out
    }
}

impl Default for Codec2RxAdapter {
    fn default() -> Self {
        Self::new()
    }
}

struct AdapterState {
    /// Circular buffer of output samples for playback
    out_buf: VecDeque<i16>,
    codec2: Codec2,
    end_tx: Option<Sender<()>>,
    resampler: Option<SincFixedIn<f32>>,
}

impl StreamAdapter for Codec2RxAdapter {
    fn start(&self, _handle: TxHandle) -> Result<(), AdapterError> {
        let (end_tx, end_rx) = channel();
        let (setup_tx, setup_rx) = channel();
        let state = self.state.clone();
        let output_card = self.output_card.clone();
        std::thread::spawn(move || stream_thread(end_rx, setup_tx, state, output_card));
        self.state.lock().unwrap().end_tx = Some(end_tx);
        // Propagate any errors arising in the thread
        let sample_rate = setup_rx.recv()??;
        debug!("selected codec2 speaker sample rate {sample_rate}");
        if sample_rate != 8000 {
            let params = SincInterpolationParameters {
                sinc_len: 256,
                f_cutoff: 0.95,
                oversampling_factor: 128,
                interpolation: rubato::SincInterpolationType::Cubic,
                window: rubato::WindowFunction::BlackmanHarris2,
            };
            // TODO: fix unwrap
            self.state.lock().unwrap().resampler =
                Some(SincFixedIn::new(sample_rate as f64 / 8000f64, 1.0, params, 160, 1).unwrap());
        }
        Ok(())
    }

    fn close(&self) -> Result<(), AdapterError> {
        let mut state = self.state.lock().unwrap();
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
            if state.out_buf.len() < 8192 {
                let mut samples = [i16::EQUILIBRIUM; 160]; // while assuming 3200
                state.codec2.decode(&mut samples, encoded);
                if let Some(resampler) = state.resampler.as_mut() {
                    let samples_f: Vec<f32> =
                        samples.iter().map(|s| *s as f32 / 16384.0f32).collect();
                    let res = resampler.process(&[samples_f], None).unwrap();
                    for s in &res[0] {
                        state.out_buf.push_back((s * 16383.0f32) as i16);
                    }
                } else {
                    // TODO: maybe get rid of VecDeque so we can decode directly into ring buffer?
                    for s in samples {
                        state.out_buf.push_back(s);
                    }
                }
            } else {
                debug!("out_buf overflow");
            }
        }
    }
}

fn output_cb(data: &mut [i16], state: &Mutex<AdapterState>, channels: u16) {
    let mut state = state.lock().unwrap();
    for d in data.chunks_mut(channels as usize) {
        d.fill(state.out_buf.pop_front().unwrap_or(i16::EQUILIBRIUM));
    }
}

/// Create and manage the stream from a dedicated thread since it's `!Send`
fn stream_thread(
    end: Receiver<()>,
    setup_tx: Sender<Result<u32, AdapterError>>,
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
    let config = match configs.find(|c| {
        (c.channels() == 1 || c.channels() == 2) && c.sample_format() == SampleFormat::I16
    }) {
        Some(c) => c,
        None => {
            let _ = setup_tx.send(Err(
                M17Codec2Error::SupportedOutputUnavailable(card_name).into()
            ));
            return;
        }
    };

    let target_sample_rate =
        if config.min_sample_rate().0 <= 8000 && config.max_sample_rate().0 >= 8000 {
            8000
        } else {
            config.min_sample_rate().0
        };
    let channels = config.channels();

    let config = config.with_sample_rate(SampleRate(target_sample_rate));
    let stream = match device.build_output_stream(
        &config.into(),
        move |data: &mut [i16], _info: &cpal::OutputCallbackInfo| {
            output_cb(data, &state, channels);
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
    let _ = setup_tx.send(Ok(target_sample_rate));
    let _ = end.recv();
    // it seems concrete impls of Stream have a Drop implementation that will handle termination
}
