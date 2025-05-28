use codec2::{Codec2, Codec2Mode};
use cpal::traits::DeviceTrait;
use cpal::traits::HostTrait;
use cpal::traits::StreamTrait;
use cpal::SampleFormat;
use cpal::SampleRate;
use log::debug;
use m17app::adapter::StreamAdapter;
use m17app::app::TxHandle;
use m17app::error::AdapterError;
use m17app::link_setup::LinkSetup;
use m17app::link_setup::M17Address;
use m17app::StreamFrame;
use rubato::Resampler;
use rubato::SincFixedOut;
use rubato::SincInterpolationParameters;
use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::mpsc::channel;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;
use std::time::Instant;

use crate::M17Codec2Error;

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

/// Control transmissions into a Codec2TxAdapter
#[derive(Clone)]
pub struct Ptt {
    tx: mpsc::Sender<Event>,
}

impl Ptt {
    pub fn set_ptt(&self, ptt: bool) {
        let _ = self.tx.send(if ptt { Event::PttOn } else { Event::PttOff });
    }
}

/// Use a microphone and local PTT to transmit Codec2 voice data into an M17 channel.
pub struct Codec2TxAdapter {
    input_card: Option<String>,
    event_tx: mpsc::Sender<Event>,
    event_rx: Mutex<Option<mpsc::Receiver<Event>>>,
    source: M17Address,
    destination: M17Address,
}

impl Codec2TxAdapter {
    pub fn new(source: M17Address, destination: M17Address) -> Self {
        let (event_tx, event_rx) = mpsc::channel();
        Self {
            input_card: None,
            event_tx,
            event_rx: Mutex::new(Some(event_rx)),
            source,
            destination,
        }
    }

    pub fn set_input_card<S: Into<String>>(&mut self, card_name: S) {
        self.input_card = Some(card_name.into());
    }

    pub fn ptt(&self) -> Ptt {
        Ptt {
            tx: self.event_tx.clone(),
        }
    }

    /// List sound cards supported for audio input.
    ///
    /// M17RT will handle any card with 1 or 2 channels and 16-bit output.
    pub fn supported_input_cards() -> Vec<String> {
        let mut out = vec![];
        let host = cpal::default_host();
        let Ok(input_devices) = host.input_devices() else {
            return out;
        };
        for d in input_devices {
            let Ok(mut configs) = d.supported_input_configs() else {
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

enum Event {
    PttOn,
    PttOff,
    MicSamples(Arc<[i16]>),
    Close,
}

impl StreamAdapter for Codec2TxAdapter {
    fn start(&self, handle: TxHandle) -> Result<(), AdapterError> {
        let Some(event_rx) = self.event_rx.lock().unwrap().take() else {
            return Err(M17Codec2Error::RepeatStart.into());
        };
        let event_tx = self.event_tx.clone();
        let (setup_tx, setup_rx) = channel();
        let input_card = self.input_card.clone();
        let from = self.source.clone();
        let to = self.destination.clone();
        std::thread::spawn(move || {
            stream_thread(event_tx, event_rx, setup_tx, input_card, handle, from, to)
        });
        let sample_rate = setup_rx.recv()??;
        debug!("selected codec2 microphone sample rate {sample_rate}");

        Ok(())
    }

    fn close(&self) -> Result<(), AdapterError> {
        let _ = self.event_tx.send(Event::Close);
        Ok(())
    }

    fn stream_began(&self, _link_setup: LinkSetup) {
        // not interested in incoming transmissions
    }

    fn stream_data(&self, _frame_number: u16, _is_final: bool, _data: Arc<[u8; 16]>) {
        // not interested in incoming transmissions

        // the only reason this is an adapter at all is for future "transmission aborted" feedback
        // when that's implemented by m17app
    }
}

fn stream_thread(
    event_tx: mpsc::Sender<Event>,
    event_rx: mpsc::Receiver<Event>,
    setup_tx: mpsc::Sender<Result<u32, AdapterError>>,
    input_card: Option<String>,
    handle: TxHandle,
    source: M17Address,
    destination: M17Address,
) {
    let host = cpal::default_host();
    let device = if let Some(input_card) = input_card {
        // TODO: more error handling for unwraps
        match host
            .input_devices()
            .unwrap()
            .find(|d| d.name().unwrap() == input_card)
        {
            Some(d) => d,
            None => {
                let _ = setup_tx.send(Err(M17Codec2Error::CardUnavailable(input_card).into()));
                return;
            }
        }
    } else {
        match host.default_input_device() {
            Some(d) => d,
            None => {
                let _ = setup_tx.send(Err(M17Codec2Error::DefaultCardUnavailable.into()));
                return;
            }
        }
    };
    let card_name = device.name().unwrap();
    let mut configs = match device.supported_input_configs() {
        Ok(c) => c,
        Err(e) => {
            let _ = setup_tx.send(Err(
                M17Codec2Error::InputConfigsUnavailable(card_name, e).into()
            ));
            return;
        }
    };
    // TODO: rank these by most favourable, same for rx
    let config = match configs.find(|c| {
        (c.channels() == 1 || c.channels() == 2) && c.sample_format() == SampleFormat::I16
    }) {
        Some(c) => c,
        None => {
            let _ = setup_tx.send(Err(
                M17Codec2Error::SupportedInputUnavailable(card_name).into()
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

    let mut acc: Box<dyn Accumulator> = if target_sample_rate != 8000 {
        Box::new(ResamplingAccumulator::new(target_sample_rate as f64))
    } else {
        Box::new(DirectAccumulator::new())
    };

    let config = config.with_sample_rate(SampleRate(target_sample_rate));
    let stream = match device.build_input_stream(
        &config.into(),
        move |data: &[i16], _info: &cpal::InputCallbackInfo| {
            let mut samples = vec![];
            for d in data.chunks(channels as usize) {
                // if we were given multi-channel input we'll pick the first channel
                // TODO: configurable?
                samples.push(d[0]);
            }
            let _ = event_tx.send(Event::MicSamples(samples.into()));
        },
        |e| {
            // abort here?
            debug!("error occurred in codec2 recording: {e:?}");
        },
        None,
    ) {
        Ok(s) => s,
        Err(e) => {
            let _ = setup_tx.send(Err(
                M17Codec2Error::InputStreamBuildError(card_name, e).into()
            ));
            return;
        }
    };

    let _ = setup_tx.send(Ok(target_sample_rate));
    let mut state = State::Idle;
    let mut codec2 = Codec2::new(Codec2Mode::MODE_3200);
    let link_setup = LinkSetup::new_voice(&source, &destination);
    let mut lich_idx = 0;
    let mut frame_number = 0;

    // Now the main loop
    while let Ok(ev) = event_rx.recv() {
        match ev {
            Event::PttOn => {
                match state {
                    State::Idle => {
                        match stream.play() {
                            Ok(()) => (),
                            Err(_e) => {
                                // TODO: report M17Codec2Error::InputStreamPlayError(card_name, e).into()
                                break;
                            }
                        }
                        acc.reset();
                        codec2 = Codec2::new(Codec2Mode::MODE_3200);
                        state = State::StartTransmitting;
                    }
                    State::StartTransmitting => {}
                    State::Transmitting => {}
                    State::Ending => state = State::EndingWithPttRestart,
                    State::EndingWithPttRestart => {}
                }
            }
            Event::PttOff => match state {
                State::Idle => {}
                State::StartTransmitting => state = State::Idle,
                State::Transmitting => state = State::Ending,
                State::Ending => {}
                State::EndingWithPttRestart => state = State::Ending,
            },
            Event::MicSamples(samples) => {
                match state {
                    State::Idle => {}
                    State::StartTransmitting
                    | State::Transmitting
                    | State::Ending
                    | State::EndingWithPttRestart => {
                        acc.handle_samples(&samples);
                        while let Some(frame) = acc.try_next_frame() {
                            let mut stream_data = [0u8; 16];
                            codec2.encode(&mut stream_data[0..8], &frame[0..160]);
                            codec2.encode(&mut stream_data[8..16], &frame[160..320]);

                            if state == State::StartTransmitting {
                                handle.transmit_stream_start(&link_setup);
                                lich_idx = 0;
                                frame_number = 0;
                                state = State::Transmitting;
                            }

                            let end_of_stream = state != State::Transmitting;
                            handle.transmit_stream_next(&StreamFrame {
                                lich_idx,
                                lich_part: link_setup.lich_part(lich_idx),
                                frame_number,
                                end_of_stream,
                                stream_data,
                            });
                            frame_number += 1;
                            lich_idx = (lich_idx + 1) % 6;

                            if end_of_stream {
                                break;
                            }
                        }

                        if state == State::Ending {
                            // when finished sending final stream frame
                            let _ = stream.pause();
                            state = State::Idle;
                        }

                        if state == State::EndingWithPttRestart {
                            acc.reset();
                            codec2 = Codec2::new(Codec2Mode::MODE_3200);
                            state = State::StartTransmitting;
                        }
                    }
                }
            }
            Event::Close => {
                // assume PTT etc. will clean up itself responsibly on close
                break;
            }
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum State {
    /// Waiting for PTT
    Idle,
    /// PTT engaged but we are collecting the first full frame of audio data before starting the stream
    StartTransmitting,
    /// Streaming voice frames
    Transmitting,
    /// PTT disengaged; we are collecting the next frame of audio to use as a final frame
    Ending,
    /// PTT was re-enaged while ending; we will send final frame then go back to StartTransmitting
    EndingWithPttRestart,
}

fn resampler_params() -> SincInterpolationParameters {
    SincInterpolationParameters {
        sinc_len: 256,
        f_cutoff: 0.95,
        oversampling_factor: 128,
        interpolation: rubato::SincInterpolationType::Cubic,
        window: rubato::WindowFunction::BlackmanHarris2,
    }
}

trait Accumulator {
    fn handle_samples(&mut self, samples: &[i16]);
    /// Return 320 samples, enough for two Codec2 frames
    fn try_next_frame(&mut self) -> Option<Vec<i16>>;
    fn reset(&mut self);
}

struct DirectAccumulator {
    buffer: Vec<i16>,
}

impl DirectAccumulator {
    fn new() -> Self {
        Self { buffer: Vec::new() }
    }
}

impl Accumulator for DirectAccumulator {
    fn handle_samples(&mut self, samples: &[i16]) {
        self.buffer.extend_from_slice(samples);
    }

    fn try_next_frame(&mut self) -> Option<Vec<i16>> {
        if self.buffer.len() >= 320 {
            let part = self.buffer.split_off(320);
            Some(std::mem::replace(&mut self.buffer, part))
        } else {
            None
        }
    }

    fn reset(&mut self) {
        self.buffer.clear();
    }
}

struct ResamplingAccumulator {
    input_rate: f64,
    buffer: Vec<i16>,
    resampler: SincFixedOut<f32>,
}

impl ResamplingAccumulator {
    fn new(input_rate: f64) -> Self {
        Self {
            input_rate,
            buffer: Vec::new(),
            resampler: make_resampler(input_rate),
        }
    }
}

impl Accumulator for ResamplingAccumulator {
    fn handle_samples(&mut self, samples: &[i16]) {
        self.buffer.extend_from_slice(samples);
    }

    fn try_next_frame(&mut self) -> Option<Vec<i16>> {
        let required = self.resampler.input_frames_next();
        if self.buffer.len() >= required {
            let mut part = self.buffer.split_off(required);
            std::mem::swap(&mut self.buffer, &mut part);
            let samples_f: Vec<f32> = part.iter().map(|s| *s as f32 / 16384.0f32).collect();
            let out = self.resampler.process(&[samples_f], None).unwrap();
            Some(out[0].iter().map(|s| (*s * 16383.0f32) as i16).collect())
        } else {
            None
        }
    }

    fn reset(&mut self) {
        self.buffer.clear();
        self.resampler = make_resampler(self.input_rate);
    }
}

fn make_resampler(input_rate: f64) -> SincFixedOut<f32> {
    // want 320 samples at a time to create 2x Codec2 frames per M17 Voice frame
    SincFixedOut::new(8000f64 / input_rate, 1.0, resampler_params(), 320, 1).unwrap()
}
