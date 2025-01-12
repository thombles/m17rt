use crate::decode::{
    parse_lsf, parse_packet, parse_stream, sync_burst_correlation, SyncBurst, SYNC_THRESHOLD,
};
use crate::protocol::{Frame, LsfFrame, PacketFrame, StreamFrame};
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

pub trait Modulator {
    /// Inform the modulator how many samples remain pending for output and latency updates.
    ///
    /// For the buffer between `Modulator` and the process which is supplying samples to the
    /// output sound card, `samples_to_play` is the number of bytes which the modulator has
    /// provided that have not yet been picked up, and `capacity` is the maximum size we can
    /// fill this particular buffer, i.e., maximum number of samples.
    ///
    /// Furthermore we attempt to track and account for the latency between the output
    /// soundcard callback, and when those samples will actually be on the wire. CPAL helpfully
    /// gives us an estimate. The latest estimate of latency is converted to a duration in terms
    /// of number of samples and provided as `output_latency`.
    ///
    /// Finally, we give the modem a monotonic timer expressed in the number of samples of time
    /// that have elapsed since the modem began operation. When the modulator advises the TNC
    /// exactly when a transmission is expected to complete, it should be expressed in terms of
    /// this number.
    ///
    /// Call this whenever bytes have been read out of the buffer, or the TNC may have something
    /// new to send.
    fn update_output_buffer(
        &mut self,
        samples_to_play: usize,
        capacity: usize,
        output_latency: usize,
        now_samples: u64,
    );

    /// Supply the next frame available from the TNC, if it was requested.
    fn next_frame(&mut self, frame: Option<ModulatorFrame>);

    /// Calculate and write out output samples for the soundcard.
    ///
    /// Returns the number of bytes valid in `out`. Should generally be called in a loop until
    /// 0 is returned.
    fn read_output_samples(&mut self, out: &mut [i16]) -> usize;

    /// Run the modulator and receive actions to process.
    ///
    /// Should be called in a loop until it returns `None`.
    fn run(&mut self) -> Option<ModulatorAction>;
}

pub enum ModulatorAction {
    /// If true, once all samples have been exhausted output should revert to equilibrium.
    ///
    /// If false, failure to pick up enough samples for output sound card is an underrun error.
    SetIdle(bool),

    /// Check with the TNC if there is a frame available for transmission.
    ///
    /// Call `next_frame()` with either the next frame, or `None` if TNC has nothing more to offer.
    GetNextFrame,

    /// Modulator wishes to send samples to the output buffer - call `read_output_samples`.
    ReadOutput,

    /// Advise the TNC that we will complete sending End Of Transmission at the given time and
    TransmissionWillEnd(u64),
}

/// Frames for transmission, emitted by the TNC and received by the Modulator.
///
/// The TNC is responsible for all timing decisions, making sure these frames are emitted in the
/// correct order, breaks between transmissions, PTT and CSMA. If the modulator is given a
/// `ModulatorFrame` value, its job is to transmit it immediately by modulating it into the output
/// buffer, or otherwise directly after any previously-supplied frames.
///
/// The modulator controls the rate at which frames are drawn out of the TNC. Therefore if the send
/// rate is too high (or there is too much channel activity) then the effect of this backpressure is
/// that the TNC's internal queues will overflow and it will either discard earlier frames in the
/// current stream, or some packets awaiting transmission.
pub enum ModulatorFrame {
    Preamble {
        /// TNC's configured TxDelay setting, increments of 10ms.
        ///
        /// TNC fires PTT and it's up to modulator to apply the setting, taking advantage of whatever
        /// buffering already exists in the sound card to reduce the artificial delay.
        tx_delay: u8,
    },
    Lsf(LsfFrame),
    Stream(StreamFrame),
    Packet(PacketFrame),
    // TODO: BertFrame
    EndOfTransmission,
}

struct SoftModulator {
    /// Next modulated frame to output - 1920 samples for 40ms frame plus 80 for ramp-up/ramp-down
    next_transmission: [i16; 2000],
    /// How much of next_transmission should in fact be transmitted
    next_len: usize,
    /// How much of next_transmission has been read out
    next_read: usize,
    /// How many pending zero samples to emit to align start of preamble with PTT taking effect
    tx_delay_padding: usize,

    /// Do we need to update idle state?
    update_idle: bool,
    /// What is that idle status?
    idle: bool,

    /// Do we need to report that a transmission end time? (True after we encoded an EOT)
    report_tx_end: bool,
    /// What will that end time be?
    tx_end_time: u64,

    /// Circular buffer of incoming samples for calculating the RRC filtered value
    filter_win: [i16; 81],
    /// Current position in filter_win
    filter_cursor: usize,

    /// Should we ask the TNC for another frame. True after each call to update_output_buffer.
    try_get_frame: bool,

    now: u64,
    output_latency: usize,
    samples_in_buf: usize,
    buf_capacity: usize,
}

impl Modulator for SoftModulator {
    fn update_output_buffer(
        &mut self,
        samples_to_play: usize,
        capacity: usize,
        output_latency: usize,
        now_samples: u64,
    ) {
        self.now = now_samples;
        self.output_latency = output_latency;
        self.buf_capacity = capacity;
        self.samples_in_buf = samples_to_play;

        if capacity - samples_to_play >= 2000 {
            self.try_get_frame = true;
        }
    }

    fn next_frame(&mut self, frame: Option<ModulatorFrame>) {
        match frame {
            Some(_f) => {}
            None => {
                self.try_get_frame = false;
            }
        }

        // this is where we write it to next_transmission

        // this is where we set report_tx_end
        todo!()
    }

    fn read_output_samples(&mut self, out: &mut [i16]) -> usize {
        let mut written = 0;

        // if we have pre-TX padding to accommodate TxDelay then expend that first
        if self.tx_delay_padding > 0 {
            let len = out.len().max(self.tx_delay_padding);
            self.tx_delay_padding -= len;
            for x in 0..len {
                out[x] = 0;
            }
            written += len;
        }

        // then follow it with whatever might be left in next_transmission
        let next_remaining = self.next_len - self.next_read;
        if next_remaining > 0 {
            let len = (out.len() - written).max(next_remaining);
            out[written..(written + len)]
                .copy_from_slice(&self.next_transmission[self.next_read..(self.next_read + len)]);
            self.next_read += len;
            written += len;
        }

        written
    }

    fn run(&mut self) -> Option<ModulatorAction> {
        if self.next_read < self.next_len {
            return Some(ModulatorAction::ReadOutput);
        }

        if self.update_idle {
            self.update_idle = false;
            return Some(ModulatorAction::SetIdle(self.idle));
        }

        if self.report_tx_end {
            self.report_tx_end = false;
            return Some(ModulatorAction::TransmissionWillEnd(self.tx_end_time));
        }

        if self.try_get_frame {
            return Some(ModulatorAction::GetNextFrame);
        }

        None
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
