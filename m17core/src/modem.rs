use crate::decode::{
    parse_lsf, parse_packet, parse_stream, sync_burst_correlation, SyncBurst, SYNC_THRESHOLD,
};
use crate::protocol::Frame;
use crate::shaping::RRC_48K;
use log::debug;

pub trait Demodulator {
    fn demod(&mut self, sample: i16) -> Option<Frame>;
    fn data_carrier_detect(&self) -> bool;
}

/// Converts a sequence of samples into frames.
pub struct SoftDemodulator {
    /// Circular buffer of incoming samples for calculating the RRC filtered value
    filter_win: [i16; 81],
    /// Current position in filter_win
    filter_cursor: usize,
    /// Circular buffer of shaped samples for performing decodes based on the last 192 symbols
    rx_win: [f32; 1920],
    /// Current position in rx_cursor
    rx_cursor: usize,
    /// A position that we are considering decoding due to decent sync
    candidate: Option<DecodeCandidate>,
    /// How many samples have we received?
    sample: u64,
    /// Remaining samples to ignore so once we already parse a frame we flush it out in full
    suppress: u16,
}

impl SoftDemodulator {
    pub fn new() -> Self {
        SoftDemodulator {
            filter_win: [0i16; 81],
            filter_cursor: 0,
            rx_win: [0f32; 1920],
            rx_cursor: 0,
            candidate: None,
            sample: 0,
            suppress: 0,
        }
    }
}

impl Demodulator for SoftDemodulator {
    fn demod(&mut self, sample: i16) -> Option<Frame> {
        self.filter_win[self.filter_cursor] = sample;
        self.filter_cursor = (self.filter_cursor + 1) % 81;
        let mut out: f32 = 0.0;
        for i in 0..81 {
            let filter_idx = (self.filter_cursor + i) % 81;
            out += RRC_48K[i] * self.filter_win[filter_idx] as f32;
        }

        self.rx_win[self.rx_cursor] = out;
        self.rx_cursor = (self.rx_cursor + 1) % 1920;

        self.sample += 1;

        if self.suppress > 0 {
            self.suppress -= 1;
            return None;
        }

        let mut burst_window = [0f32; 71];
        for i in 0..71 {
            let c = (self.rx_cursor + i) % 1920;
            burst_window[i] = self.rx_win[c];
        }

        for burst in [
            SyncBurst::Lsf,
            SyncBurst::Bert,
            SyncBurst::Stream,
            SyncBurst::Packet,
        ] {
            let (diff, max, shift) = sync_burst_correlation(burst.target(), &burst_window);
            if diff < SYNC_THRESHOLD {
                let mut new_candidate = true;
                if let Some(c) = self.candidate.as_mut() {
                    if diff > c.diff {
                        c.age += 1;
                        new_candidate = false;
                    }
                }
                if new_candidate {
                    self.candidate = Some(DecodeCandidate {
                        burst,
                        age: 1,
                        diff,
                        gain: max,
                        shift,
                    });
                }
            }
            if diff >= SYNC_THRESHOLD
                && self
                    .candidate
                    .as_ref()
                    .map(|c| c.burst == burst)
                    .unwrap_or(false)
            {
                if let Some(c) = self.candidate.take() {
                    let start_idx = self.rx_cursor + 1920 - (c.age as usize);
                    let start_sample = self.sample - c.age as u64;
                    let mut pkt_samples = [0f32; 192];
                    for i in 0..192 {
                        let rx_idx = (start_idx + i * 10) % 1920;
                        pkt_samples[i] = (self.rx_win[rx_idx] - c.shift) / c.gain;
                    }
                    match c.burst {
                        SyncBurst::Lsf => {
                            debug!(
                                "Found LSF at sample {} diff {} max {} shift {}",
                                start_sample, c.diff, c.gain, c.shift
                            );
                            if let Some(frame) = parse_lsf(&pkt_samples) {
                                self.suppress = 191 * 10;
                                return Some(Frame::Lsf(frame));
                            }
                        }
                        SyncBurst::Bert => {
                            debug!("Found BERT at sample {} diff {}", start_sample, c.diff);
                        }
                        SyncBurst::Stream => {
                            debug!(
                                "Found STREAM at sample {} diff {} max {} shift {}",
                                start_sample, c.diff, c.gain, c.shift
                            );
                            if let Some(frame) = parse_stream(&pkt_samples) {
                                self.suppress = 191 * 10;
                                return Some(Frame::Stream(frame));
                            }
                        }
                        SyncBurst::Packet => {
                            debug!("Found PACKET at sample {} diff {}", start_sample, c.diff);
                            if let Some(frame) = parse_packet(&pkt_samples) {
                                self.suppress = 191 * 10;
                                return Some(Frame::Packet(frame));
                            }
                        }
                    }
                }
            }
        }

        None
    }

    fn data_carrier_detect(&self) -> bool {
        false
    }
}

impl Default for SoftDemodulator {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug)]
pub(crate) struct DecodeCandidate {
    burst: SyncBurst,
    age: u8,
    diff: f32,
    gain: f32,
    shift: f32,
}
