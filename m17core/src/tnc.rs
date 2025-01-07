use crate::kiss::{KissBuffer, KissFrame};
use crate::protocol::{Frame, LichCollection, LsfFrame, Mode, PacketFrameCounter};

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
}

impl SoftTnc {
    pub fn new() -> Self {
        Self {
            kiss_buffer: KissBuffer::new(),
            outgoing_kiss: None,
            state: State::Idle,
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
                            if lsf.crc() == 0 {
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

    /// Update the number of samples that have been received by the incoming stream, as a form of timekeeping
    pub fn advance_samples(&mut self, _samples: u64) {}

    pub fn set_data_carrier_detect(&mut self, _dcd: bool) {}

    pub fn read_tx_frame(&mut self) -> Result<Option<Frame>, SoftTncError> {
        // yes we want to deal with Frames here
        // it's important to establish successful decode that SoftDemodulator is aware of the frame innards
        Ok(None)
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

    pub fn write_kiss(&mut self, buf: &[u8]) -> usize {
        let target_buf = self.kiss_buffer.buf_remaining();
        let n = buf.len().min(target_buf.len());
        target_buf[0..n].copy_from_slice(&buf[0..n]);
        self.kiss_buffer.did_write(n);
        while let Some(_kiss_frame) = self.kiss_buffer.next_frame() {
            // TODO: handle host-to-TNC message
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
    /// Nothing happening.
    Idle,

    /// We received some stream data but missed the leading LSF so we are trying to assemble from LICH.
    RxAcquiringStream(RxAcquiringStreamState),

    /// We have acquired an identified stream transmission and are sending data payloads to the host.
    RxStream(RxStreamState),

    /// We are receiving a packet. All is well so far, and there is more data to come before we tell the host.
    RxPacket(RxPacketState),
    // TODO: TX
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
