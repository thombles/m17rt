use codec2::{Codec2, Codec2Mode};
use cpal::traits::DeviceTrait;
use cpal::traits::HostTrait;
use cpal::traits::StreamTrait;
use cpal::{Sample, SampleFormat, SampleRate};
use log::debug;
use m17app::adapter::StreamAdapter;
use m17app::app::TxHandle;
use m17core::address::Address;
use m17core::address::Callsign;
use m17core::protocol::LsfFrame;
use m17core::protocol::StreamFrame;
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

pub struct Codec2Adapter {
    state: Arc<Mutex<AdapterState>>,
    // TODO: make this configurable
    output_card: String,
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
            output_card: "default".to_owned(),
        }
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
    fn adapter_registered(&self, _id: usize, handle: TxHandle) {
        self.state.lock().unwrap().tx = Some(handle);

        let (end_tx, end_rx) = channel();
        let state = self.state.clone();
        let output_card = self.output_card.clone();
        std::thread::spawn(move || stream_thread(end_rx, state, output_card));
        self.state.lock().unwrap().end_tx = Some(end_tx);
    }

    fn adapter_removed(&self) {
        let mut state = self.state.lock().unwrap();
        state.tx = None;
        state.end_tx = None;
    }

    fn tnc_started(&self) {}

    fn tnc_closed(&self) {}

    fn stream_began(&self, _lsf: LsfFrame) {
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
fn stream_thread(end: Receiver<()>, state: Arc<Mutex<AdapterState>>, output_card: String) {
    let host = cpal::default_host();
    let device = host
        .output_devices()
        .unwrap()
        .find(|d| d.name().unwrap() == output_card)
        .unwrap();
    let mut configs = device.supported_output_configs().unwrap();
    // TODO: channels == 1 doesn't work on a Raspberry Pi
    // make this configurable and support interleaving LRLR stereo samples if using 2 channels
    let config = configs
        .find(|c| c.channels() == 1 && c.sample_format() == SampleFormat::I16)
        .unwrap()
        .with_sample_rate(SampleRate(8000));
    let stream = device
        .build_output_stream(
            &config.into(),
            move |data: &mut [i16], _info: &cpal::OutputCallbackInfo| {
                output_cb(data, &state);
            },
            |e| {
                // trigger end_tx here? always more edge cases
                debug!("error occurred in codec2 playback: {e:?}");
            },
            None,
        )
        .unwrap();
    stream.play().unwrap();
    let _ = end.recv();
    // it seems concrete impls of Stream have a Drop implementation that will handle termination
}

pub struct WavePlayer;

impl WavePlayer {
    pub fn play(path: PathBuf, tx: TxHandle) {
        let mut reader = hound::WavReader::open(path).unwrap();
        let mut samples = reader.samples::<i16>();

        let mut codec = Codec2::new(Codec2Mode::MODE_3200);
        let mut in_buf = [0i16; 160];
        let mut out_buf = [0u8; 16];
        let mut lsf_chunk: usize = 0;
        const TICK: Duration = Duration::from_millis(40);
        let mut next_tick = Instant::now() + TICK;
        let mut frame_number = 0;

        // TODO: need a better way to create addresses from std strings

        let lsf = LsfFrame::new_voice(
            &Address::Callsign(Callsign(b"VK7XT    ".clone())),
            &Address::Broadcast,
        );

        tx.transmit_stream_start(lsf.clone());

        loop {
            let mut last_one = false;
            for mut out in out_buf.chunks_mut(8) {
                for i in 0..160 {
                    let sample = match samples.next() {
                        Some(Ok(sample)) => sample,
                        _ => {
                            last_one = true;
                            0
                        }
                    };
                    in_buf[i] = sample;
                }
                codec.encode(&mut out, &in_buf);
            }
            tx.transmit_stream_next(StreamFrame {
                lich_idx: lsf_chunk as u8,
                lich_part: lsf.0[lsf_chunk * 5..(lsf_chunk + 1) * 5]
                    .try_into()
                    .unwrap(),
                frame_number,
                end_of_stream: last_one,
                stream_data: out_buf.clone(),
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
