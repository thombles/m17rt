use crate::address::{Address, Callsign};
use crate::kiss::{KissBuffer, KissFrame, PORT_PACKET_BASIC, PORT_PACKET_FULL, PORT_STREAM};
use crate::modem::ModulatorFrame;
use crate::protocol::{
    Frame, LichCollection, LsfFrame, Mode, PacketFrame, PacketFrameCounter, StreamFrame,
};

/// Handles the KISS protocol and frame management for `SoftModulator` and `SoftDemodulator`.
///
/// These components work alongside each other. User is responsible for chaining them together
/// or doing something else with the data.
pub struct SoftTnc {
    /// Handle framing of KISS commands from the host, which may arrive in arbitrary binary blobs.
    kiss_buffer: KissBuffer,

    /// Kiss message that needs to be sent to the host.
    outgoing_kiss: Option<OutgoingKiss>,

    /// Current RX or TX function of the TNC.
    state: State,

    /// Latest state of data carrier detect from demodulator - controls whether we can go to TX
    dcd: bool,

    /// If CSMA declined to transmit into an idle slot, at what point do we next check it?
    next_csma_check: Option<u64>,

    /// Current monotonic time, counted in samples
    now: u64,

    // TODO: use a static ring buffer crate of some sort?
    /// Circular buffer of packets enqueued for transmission
    packet_queue: [PendingPacket; 4],

    /// Next slot to fill
    packet_next: usize,

    /// Current packet index, which is either partly transmitted or not transmitted at all.
    packet_curr: usize,

    /// If true, packet_next == packet_curr implies full queue. packet_next is invalid.
    /// If false, it implies empty queue.
    packet_full: bool,

    /// The LSF for a stream we are going to start transmitting.
    ///
    /// This serves as a general indicator that we want to tx a stream.
    stream_pending_lsf: Option<LsfFrame>,

    /// Circular buffer of stream data enqueued for transmission.
    ///
    /// When the queue empties out, we hope that the last one has the end-of-stream flag set.
    /// Otherwise a buffer underrun has occurred.
    ///
    /// Overruns are less troublesome - we can drop frames and receiving stations should cope.
    stream_queue: [StreamFrame; 8],

    /// Next slot to fill
    stream_next: usize,

    /// Current unsent stream frame index
    stream_curr: usize,

    /// True if stream_next == stream_curr because the queue is full. stream_next is invalid.
    stream_full: bool,

    /// Should PTT be on right now? Polled by external
    ptt: bool,
}

impl SoftTnc {
    pub fn new() -> Self {
        Self {
            kiss_buffer: KissBuffer::new(),
            outgoing_kiss: None,
            state: State::Idle,
            dcd: false,
            next_csma_check: None,
            now: 0,
            packet_queue: Default::default(),
            packet_next: 0,
            packet_curr: 0,
            packet_full: false,
            stream_pending_lsf: None,
            stream_queue: Default::default(),
            stream_next: 0,
            stream_curr: 0,
            stream_full: false,
            ptt: false,
        }
    }

    /// Process an individual `Frame` that has been decoded by the modem.
    pub fn handle_frame(&mut self, frame: Frame) {
        match frame {
            Frame::Lsf(lsf) => {
                // A new LSF implies a clean slate.
                // If we were partway through decoding something else then we missed it.
                match lsf.mode() {
                    Mode::Packet => {
                        self.state = State::RxPacket(RxPacketState {
                            lsf,
                            packet: [0u8; 825],
                            count: 0,
                        })
                    }
                    Mode::Stream => {
                        let kiss = KissFrame::new_stream_setup(&lsf.0).unwrap();
                        self.kiss_to_host(kiss);
                        self.state = State::RxStream(RxStreamState { lsf, index: 0 });
                    }
                }
            }
            Frame::Packet(packet) => {
                match &mut self.state {
                    State::RxPacket(ref mut rx) => {
                        match packet.counter {
                            PacketFrameCounter::Frame { index } => {
                                if index == rx.count && index < 32 {
                                    let start = 25 * index;
                                    rx.packet[start..(start + 25)].copy_from_slice(&packet.payload);
                                    rx.count += 1;
                                } else {
                                    // unexpected order - something has gone wrong
                                    self.state = State::Idle;
                                }
                            }
                            PacketFrameCounter::FinalFrame { payload_len } => {
                                let start = 25 * rx.count;
                                let end = start + payload_len;
                                rx.packet[start..(start + payload_len)]
                                    .copy_from_slice(&packet.payload);
                                // TODO: compatible packets should be sent on port 0 too
                                let kiss =
                                    KissFrame::new_full_packet(&rx.lsf.0, &rx.packet[0..end])
                                        .unwrap();
                                self.kiss_to_host(kiss);
                                self.state = State::Idle;
                            }
                        }
                    }
                    _ => {
                        // Invalid transition
                        self.state = State::Idle;
                    }
                }
            }
            Frame::Stream(stream) => {
                match &mut self.state {
                    State::RxStream(ref mut rx) => {
                        // TODO: consider wraparound from 0x7fff
                        if stream.frame_number < rx.index {
                            let mut lich = LichCollection::new();
                            lich.set_segment(stream.lich_idx, stream.lich_part);
                            self.state = State::RxAcquiringStream(RxAcquiringStreamState { lich });
                        } else {
                            rx.index = stream.frame_number + 1;
                            let kiss = KissFrame::new_stream_data(&stream).unwrap();
                            self.kiss_to_host(kiss);
                            // TODO: end stream if LICH updates indicate non-META part has changed
                            // (this implies a new station)
                            if stream.end_of_stream {
                                self.state = State::Idle;
                            }
                        }
                    }
                    State::RxAcquiringStream(ref mut rx) => {
                        rx.lich.set_segment(stream.lich_idx, stream.lich_part);
                        if let Some(maybe_lsf) = rx.lich.try_assemble() {
                            let lsf = LsfFrame(maybe_lsf);
                            // LICH can change mid-transmission so wait until the CRC is correct
                            // to ensure (to high probability) we haven't done a "torn read"
                            if lsf.check_crc() == 0 {
                                let kiss = KissFrame::new_stream_setup(&lsf.0).unwrap();
                                self.kiss_to_host(kiss);
                                // TODO: avoid discarding the first data payload here
                                // need a queue depth of 2 for outgoing kiss
                                self.state = State::RxStream(RxStreamState {
                                    lsf,
                                    index: stream.frame_number + 1,
                                });
                            }
                        }
                    }
                    _ => {
                        // If coming from another state, we have missed something.
                        // Never mind, let's start tracking LICH.
                        let mut lich = LichCollection::new();
                        lich.set_segment(stream.lich_idx, stream.lich_part);
                        self.state = State::RxAcquiringStream(RxAcquiringStreamState { lich })
                    }
                }
            }
        }
    }

    pub fn set_data_carrier_detect(&mut self, dcd: bool) {
        self.dcd = dcd;
    }

    pub fn set_now(&mut self, now_samples: u64) {
        self.now = now_samples;
        match self.state {
            State::TxEndingAtTime(time) => {
                if now_samples >= time {
                    self.ptt = false;
                    self.state = State::Idle;
                }
            }
            _ => (),
        }
    }

    pub fn ptt(&self) -> bool {
        self.ptt
    }

    pub fn set_tx_end_time(&mut self, in_samples: usize) {
        log::debug!("tnc has been told that tx will complete in {in_samples} samples");
        match self.state {
            State::TxEnding => {
                self.state = State::TxEndingAtTime(self.now + in_samples as u64);
            }
            _ => (),
        }
    }

    pub fn read_tx_frame(&mut self) -> Option<ModulatorFrame> {
        match self.state {
            State::Idle | State::RxAcquiringStream(_) | State::RxStream(_) | State::RxPacket(_) => {
                let stream_wants_to_tx = self.stream_pending_lsf.is_some();
                let packet_wants_to_tx = self.packet_full || (self.packet_next != self.packet_curr);
                if !stream_wants_to_tx && !packet_wants_to_tx {
                    return None;
                }

                // We have something we might send if the channel is free
                match self.next_csma_check {
                    None => {
                        if self.dcd {
                            self.next_csma_check = Some(self.now + 1920);
                            return None;
                        } else {
                            // channel is idle at the moment we get a frame to send
                            // go right ahead
                        }
                    }
                    Some(at_time) => {
                        if self.now < at_time {
                            return None;
                        }
                        // 25% chance that we'll transmit this slot.
                        // Using self.now as random is probably fine so long as it's not being set in
                        // a lumpy manner. m17app's soundmodem should be fine.
                        // TODO: bring in prng to help in cases where `now` never ends in 0b11
                        let p1_4 = (self.now & 3) == 3;
                        if !self.dcd || !p1_4 {
                            self.next_csma_check = Some(self.now + 1920);
                            return None;
                        } else {
                            self.next_csma_check = None;
                        }
                    }
                }

                if stream_wants_to_tx {
                    self.state = State::TxStream;
                } else {
                    self.state = State::TxPacket;
                }
                self.ptt = true;
                // TODO: true txdelay
                Some(ModulatorFrame::Preamble { tx_delay: 0 })
            }
            State::TxStream => {
                if !self.stream_full && self.stream_next == self.stream_curr {
                    return None;
                }
                if let Some(lsf) = self.stream_pending_lsf.take() {
                    return Some(ModulatorFrame::Lsf(lsf));
                }
                let frame = self.stream_queue[self.stream_curr].clone();
                if self.stream_full {
                    self.stream_full = false;
                }
                self.stream_curr = (self.stream_curr + 1) % 8;
                if frame.end_of_stream {
                    self.state = State::TxStreamSentEndOfStream;
                }
                Some(ModulatorFrame::Stream(frame))
            }
            State::TxStreamSentEndOfStream => {
                self.state = State::TxEnding;
                Some(ModulatorFrame::EndOfTransmission)
            }
            State::TxPacket => {
                if !self.packet_full && self.packet_next == self.packet_curr {
                    return None;
                }
                while self.packet_next != self.packet_curr {
                    match self.packet_queue[self.packet_curr].next_frame() {
                        Some(frame) => {
                            return Some(frame);
                        }
                        None => {
                            self.packet_curr = (self.packet_curr + 1) % 4;
                        }
                    }
                }
                self.state = State::TxEnding;
                Some(ModulatorFrame::EndOfTransmission)
            }
            State::TxEnding | State::TxEndingAtTime(_) => {
                // Once we have signalled EOT we withold any new frames until
                // the channel fully clears and we are ready to TX again
                None
            }
        }
    }

    /// Read KISS message to be sent to host.
    ///
    /// After each frame input, this should be consumed in a loop until length 0 is returned.
    /// This component will never block. Upstream interface can provide blocking `read()` if desired.
    pub fn read_kiss(&mut self, target_buf: &mut [u8]) -> usize {
        match self.outgoing_kiss.as_mut() {
            Some(outgoing) => {
                let n = (outgoing.kiss_frame.len - outgoing.sent).min(target_buf.len());
                target_buf[0..n]
                    .copy_from_slice(&outgoing.kiss_frame.data[outgoing.sent..(outgoing.sent + n)]);
                outgoing.sent += n;
                if outgoing.sent == outgoing.kiss_frame.len {
                    self.outgoing_kiss = None;
                }
                n
            }
            None => 0,
        }
    }

    /// Host sends in some KISS data.
    pub fn write_kiss(&mut self, buf: &[u8]) -> usize {
        let target_buf = self.kiss_buffer.buf_remaining();
        let n = buf.len().min(target_buf.len());
        target_buf[0..n].copy_from_slice(&buf[0..n]);
        self.kiss_buffer.did_write(n);
        while let Some(kiss_frame) = self.kiss_buffer.next_frame() {
            let Ok(port) = kiss_frame.port() else {
                continue;
            };
            if port == PORT_PACKET_BASIC {
                if self.packet_full {
                    continue;
                }
                let mut pending = PendingPacket::new();
                pending.app_data[0] = 0x00; // RAW
                let Ok(mut len) = kiss_frame.decode_payload(&mut pending.app_data[1..]) else {
                    continue;
                };
                len += 1; // for RAW prefix
                let packet_crc = crate::crc::m17_crc(&pending.app_data[0..len]);
                pending.app_data[len..len + 2].copy_from_slice(&packet_crc.to_be_bytes());
                pending.app_data_len = len + 2;
                pending.lsf = Some(LsfFrame::new_packet(
                    &Address::Callsign(Callsign(b"M17RT-PKT".clone())),
                    &Address::Broadcast,
                ));
                self.packet_queue[self.packet_next] = pending;
                self.packet_next = (self.packet_next + 1) % 4;
                if self.packet_next == self.packet_curr {
                    self.packet_full = true;
                }
            } else if port == PORT_PACKET_FULL {
                if self.packet_full {
                    continue;
                }
                let mut pending = PendingPacket::new();
                let mut payload = [0u8; 855];
                let Ok(len) = kiss_frame.decode_payload(&mut payload) else {
                    continue;
                };
                if len < 33 {
                    continue;
                }
                let mut lsf = LsfFrame([0u8; 30]);
                lsf.0.copy_from_slice(&payload[0..30]);
                if lsf.check_crc() != 0 {
                    continue;
                }
                pending.lsf = Some(lsf);
                let app_data_len = len - 30;
                pending.app_data[0..app_data_len].copy_from_slice(&payload[30..]);
                pending.app_data_len = app_data_len;
                self.packet_queue[self.packet_next] = pending;
                self.packet_next = (self.packet_next + 1) % 4;
                if self.packet_next == self.packet_curr {
                    self.packet_full = true;
                }
            } else if port == PORT_STREAM {
                let mut payload = [0u8; 30];
                let Ok(len) = kiss_frame.decode_payload(&mut payload) else {
                    continue;
                };
                if len < 26 {
                    log::debug!("payload len too short");
                    continue;
                }
                if len == 30 {
                    let lsf = LsfFrame(payload);
                    if lsf.check_crc() != 0 {
                        continue;
                    }
                    self.stream_pending_lsf = Some(lsf);
                } else {
                    if self.stream_full {
                        log::debug!("stream full");
                        continue;
                    }
                    let frame_num_part = u16::from_be_bytes([payload[6], payload[7]]);
                    self.stream_queue[self.stream_next] = StreamFrame {
                        lich_idx: payload[5] >> 5,
                        lich_part: payload[0..5].try_into().unwrap(),
                        frame_number: frame_num_part & 0x7fff,
                        end_of_stream: frame_num_part & 0x8000 > 0,
                        stream_data: payload[8..24].try_into().unwrap(),
                    };
                    self.stream_next = (self.stream_next + 1) % 8;
                    if self.stream_next == self.stream_curr {
                        self.stream_full = true;
                    }
                }
            }
        }
        n
    }

    fn kiss_to_host(&mut self, kiss_frame: KissFrame) {
        self.outgoing_kiss = Some(OutgoingKiss {
            kiss_frame,
            sent: 0,
        });
    }
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum SoftTncError {
    General(&'static str),
    InvalidState,
}

struct OutgoingKiss {
    kiss_frame: KissFrame,
    sent: usize,
}

enum State {
    /// Nothing happening. We may have TX data queued but we won't act on it until CSMA opens up.
    Idle,

    /// We received some stream data but missed the leading LSF so we are trying to assemble from LICH.
    RxAcquiringStream(RxAcquiringStreamState),

    /// We have acquired an identified stream transmission and are sending data payloads to the host.
    RxStream(RxStreamState),

    /// We are receiving a packet. All is well so far, and there is more data to come before we tell the host.
    RxPacket(RxPacketState),

    /// PTT is on and this is a stream-type transmission. New data may be added.
    TxStream,

    /// We have delivered the last frame in the current stream
    TxStreamSentEndOfStream,

    /// PTT is on and this is a packet-type transmission. New packets may be enqueued.
    TxPacket,

    /// We gave modulator an EndOfTransmission. PTT is still on, waiting for modulator to advise end time.
    TxEnding,

    /// Ending transmission, PTT remains on, but we know the timestamp at which we should disengage it.
    TxEndingAtTime(u64),
}

struct RxAcquiringStreamState {
    /// Partial assembly of LSF by accumulating LICH fields.
    lich: LichCollection,
}

struct RxStreamState {
    /// Track identifying information for this transmission so we can tell if it changes.
    lsf: LsfFrame,

    /// Expected next frame number. Allowed to skip values on RX, but not go backwards.
    index: u16,
}

struct RxPacketState {
    /// Initial LSF
    lsf: LsfFrame,

    /// Accumulation of packet data that we have received so far.
    packet: [u8; 825],

    /// Number of payload frames we have received. If we are stably in the RxPacket state,
    /// this will be between 0 and 32 inclusive.
    count: usize,
}

struct PendingPacket {
    lsf: Option<LsfFrame>,

    app_data: [u8; 825],
    app_data_len: usize,
    app_data_transmitted: usize,
}

impl PendingPacket {
    fn new() -> Self {
        Self {
            lsf: None,
            app_data: [0u8; 825],
            app_data_len: 0,
            app_data_transmitted: 0,
        }
    }

    /// Returns next frame, not including preamble or EOT.
    ///
    /// False means all data frames have been sent.
    fn next_frame(&mut self) -> Option<ModulatorFrame> {
        if let Some(lsf) = self.lsf.take() {
            return Some(ModulatorFrame::Lsf(lsf));
        }
        if self.app_data_len == self.app_data_transmitted {
            return None;
        }
        let remaining = self.app_data_len - self.app_data_transmitted;
        let (counter, data_len) = if remaining <= 25 {
            (
                PacketFrameCounter::FinalFrame {
                    payload_len: remaining,
                },
                remaining,
            )
        } else {
            (
                PacketFrameCounter::Frame {
                    index: self.app_data_transmitted / 25,
                },
                25,
            )
        };
        let mut payload = [0u8; 25];
        payload.copy_from_slice(
            &self.app_data[self.app_data_transmitted..(self.app_data_transmitted + data_len)],
        );
        self.app_data_transmitted += data_len;
        Some(ModulatorFrame::Packet(PacketFrame { payload, counter }))
    }
}

impl Default for PendingPacket {
    fn default() -> Self {
        Self {
            lsf: None,
            app_data: [0u8; 825],
            app_data_len: 0,
            app_data_transmitted: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kiss::{KissCommand, PORT_STREAM};
    use crate::protocol::StreamFrame;

    // TODO: finish all handle_frame tests as below
    // this will be much more straightforward when we have a way to create LSFs programatically

    // receiving a single-frame packet

    // receiving a multi-frame packet

    // part of one packet and then another

    #[test]
    fn tnc_receive_stream() {
        let lsf = LsfFrame([
            255, 255, 255, 255, 255, 255, 0, 0, 0, 159, 221, 81, 5, 5, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 131, 53,
        ]);
        let stream1 = StreamFrame {
            lich_idx: 0,
            lich_part: [255, 255, 255, 255, 255],
            frame_number: 0,
            end_of_stream: false,
            stream_data: [
                128, 0, 119, 115, 220, 252, 41, 235, 8, 0, 116, 195, 94, 244, 45, 75,
            ],
        };
        let stream2 = StreamFrame {
            lich_idx: 1,
            lich_part: [255, 0, 0, 0, 159],
            frame_number: 1,
            end_of_stream: true,
            stream_data: [
                17, 0, 94, 82, 216, 135, 181, 15, 30, 0, 125, 195, 152, 183, 41, 57,
            ],
        };
        let mut tnc = SoftTnc::new();
        let mut kiss = KissFrame::new_empty();
        assert_eq!(tnc.read_kiss(&mut kiss.data), 0);

        tnc.handle_frame(Frame::Lsf(lsf));
        kiss.len = tnc.read_kiss(&mut kiss.data);
        assert_eq!(kiss.command().unwrap(), KissCommand::DataFrame);
        assert_eq!(kiss.port().unwrap(), PORT_STREAM);

        let mut payload_buf = [0u8; 2048];
        let n = kiss.decode_payload(&mut payload_buf).unwrap();
        assert_eq!(n, 30);

        tnc.handle_frame(Frame::Stream(stream1));
        kiss.len = tnc.read_kiss(&mut kiss.data);
        assert_eq!(kiss.command().unwrap(), KissCommand::DataFrame);
        assert_eq!(kiss.port().unwrap(), PORT_STREAM);

        let n = kiss.decode_payload(&mut payload_buf).unwrap();
        assert_eq!(n, 26);

        tnc.handle_frame(Frame::Stream(stream2));
        kiss.len = tnc.read_kiss(&mut kiss.data);
        assert_eq!(kiss.command().unwrap(), KissCommand::DataFrame);
        assert_eq!(kiss.port().unwrap(), PORT_STREAM);

        let n = kiss.decode_payload(&mut payload_buf).unwrap();
        assert_eq!(n, 26);
    }

    #[test]
    fn tnc_acquire_stream() {
        let frames = [
            StreamFrame {
                lich_idx: 0,
                lich_part: [255, 255, 255, 255, 255],
                frame_number: 0,
                end_of_stream: false,
                stream_data: [
                    128, 0, 119, 115, 220, 252, 41, 235, 8, 0, 116, 195, 94, 244, 45, 75,
                ],
            },
            StreamFrame {
                lich_idx: 1,
                lich_part: [255, 0, 0, 0, 159],
                frame_number: 1,
                end_of_stream: false,
                stream_data: [
                    17, 0, 94, 82, 216, 135, 181, 15, 30, 0, 125, 195, 152, 183, 41, 57,
                ],
            },
            StreamFrame {
                lich_idx: 2,
                lich_part: [221, 81, 5, 5, 0],
                frame_number: 2,
                end_of_stream: false,
                stream_data: [
                    17, 128, 93, 74, 154, 167, 169, 11, 20, 0, 116, 91, 158, 220, 45, 111,
                ],
            },
            StreamFrame {
                lich_idx: 3,
                lich_part: [0, 0, 0, 0, 0],
                frame_number: 3,
                end_of_stream: false,
                stream_data: [
                    15, 128, 114, 83, 218, 252, 59, 111, 31, 128, 116, 91, 84, 231, 45, 105,
                ],
            },
            StreamFrame {
                lich_idx: 4,
                lich_part: [0, 0, 0, 0, 0],
                frame_number: 4,
                end_of_stream: false,
                stream_data: [
                    9, 128, 119, 115, 220, 220, 57, 15, 48, 128, 124, 83, 158, 236, 181, 91,
                ],
            },
            StreamFrame {
                lich_idx: 5,
                lich_part: [0, 0, 0, 131, 53],
                frame_number: 5,
                end_of_stream: false,
                stream_data: [
                    52, 0, 116, 90, 152, 167, 225, 216, 32, 0, 116, 83, 156, 212, 33, 216,
                ],
            },
        ];

        let mut tnc = SoftTnc::new();
        let mut kiss = KissFrame::new_empty();
        for f in frames {
            tnc.handle_frame(Frame::Stream(f));
        }
        kiss.len = tnc.read_kiss(&mut kiss.data);
        let mut payload_buf = [0u8; 2048];
        let n = kiss.decode_payload(&mut payload_buf).unwrap();
        assert_eq!(n, 30);
        assert_eq!(
            &payload_buf[0..30],
            [
                255, 255, 255, 255, 255, 255, 0, 0, 0, 159, 221, 81, 5, 5, 0, 0, 0, 0, 0, 0, 0, 0,
                0, 0, 0, 0, 0, 0, 131, 53,
            ]
        );
    }

    #[test]
    fn tnc_handle_skipped_stream_frame() {
        let lsf = LsfFrame([
            255, 255, 255, 255, 255, 255, 0, 0, 0, 159, 221, 81, 5, 5, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 131, 53,
        ]);
        let stream1 = StreamFrame {
            lich_idx: 0,
            lich_part: [255, 255, 255, 255, 255],
            frame_number: 0,
            end_of_stream: false,
            stream_data: [
                128, 0, 119, 115, 220, 252, 41, 235, 8, 0, 116, 195, 94, 244, 45, 75,
            ],
        };
        let stream3 = StreamFrame {
            lich_idx: 2,
            lich_part: [221, 81, 5, 5, 0],
            frame_number: 2,
            end_of_stream: false,
            stream_data: [
                17, 128, 93, 74, 154, 167, 169, 11, 20, 0, 116, 91, 158, 220, 45, 111,
            ],
        };
        let mut tnc = SoftTnc::new();
        let mut kiss = KissFrame::new_empty();
        assert_eq!(tnc.read_kiss(&mut kiss.data), 0);

        tnc.handle_frame(Frame::Lsf(lsf));
        kiss.len = tnc.read_kiss(&mut kiss.data);
        assert_eq!(kiss.command().unwrap(), KissCommand::DataFrame);
        assert_eq!(kiss.port().unwrap(), PORT_STREAM);

        let mut payload_buf = [0u8; 2048];
        let n = kiss.decode_payload(&mut payload_buf).unwrap();
        assert_eq!(n, 30);

        tnc.handle_frame(Frame::Stream(stream1));
        kiss.len = tnc.read_kiss(&mut kiss.data);
        assert_eq!(kiss.command().unwrap(), KissCommand::DataFrame);
        assert_eq!(kiss.port().unwrap(), PORT_STREAM);

        let n = kiss.decode_payload(&mut payload_buf).unwrap();
        assert_eq!(n, 26);

        tnc.handle_frame(Frame::Stream(stream3));
        kiss.len = tnc.read_kiss(&mut kiss.data);
        assert_eq!(kiss.command().unwrap(), KissCommand::DataFrame);
        assert_eq!(kiss.port().unwrap(), PORT_STREAM);

        let n = kiss.decode_payload(&mut payload_buf).unwrap();
        assert_eq!(n, 26);
    }
}
