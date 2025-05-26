use codec2::{Codec2, Codec2Mode};
use m17app::app::TxHandle;
use m17app::link_setup::LinkSetup;
use m17app::link_setup::M17Address;
use m17app::StreamFrame;
use std::path::PathBuf;
use std::time::Duration;
use std::time::Instant;

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
