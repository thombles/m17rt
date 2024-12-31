use crate::kiss::{KissBuffer, KissFrame};
use crate::protocol::{Frame, LichCollection, LsfFrame};

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
    pub fn handle_frame(&mut self, _frame: Frame) -> Result<(), SoftTncError> {
        Ok(())
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
    pub fn read_kiss(&mut self, target_buf: &mut [u8]) -> Result<usize, SoftTncError> {
        match self.outgoing_kiss.as_mut() {
            Some(outgoing) => {
                let n = (outgoing.kiss_frame.len - outgoing.sent).min(target_buf.len());
                target_buf[0..n]
                    .copy_from_slice(&outgoing.kiss_frame.data[outgoing.sent..(outgoing.sent + n)]);
                outgoing.sent += n;
                Ok(n)
            }
            None => Ok(0),
        }
    }

    pub fn write_kiss(&mut self, buf: &[u8]) -> Result<usize, SoftTncError> {
        let target_buf = self.kiss_buffer.buf_remaining();
        let n = buf.len().min(target_buf.len());
        target_buf[0..n].copy_from_slice(&buf[0..n]);
        self.kiss_buffer.did_write(n);
        while let Some(_kiss_frame) = self.kiss_buffer.next_frame() {
            // TODO: handle host-to-TNC message
        }
        Ok(n)
    }
}

#[derive(Debug)]
pub enum SoftTncError {
    General(&'static str),
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
}

struct RxPacketState {
    /// Accumulation of packet data that we have received so far.
    packet: [u8; 825],

    /// Number of frames we have received. If we are stably in the RxPacket state,
    /// this will be between 1 and 32 inclusive. The first frame gets us into the
    /// rx state, and the maximum 33rd frame must end the transmission and state.
    count: usize,
}
