use crate::protocol::Frame;

/// Handles the KISS protocol and frame management for `SoftModulator` and `SoftDemodulator`.
///
/// These components work alongside each other. User is responsible for chaining them together
/// or doing something else with the data.
pub struct SoftTnc {}

impl SoftTnc {
    /// Process an individual `Frame` that has been decoded by the modem.
    pub fn handle_frame(&mut self, _frame: Frame) -> Result<(), SoftTncError> {
        Ok(())
    }

    ///
    pub fn advance_samples(&mut self, _samples: u64) {}

    pub fn set_data_carrier_detect(&mut self, _dcd: bool) {}

    pub fn read_tx_frame(&mut self) -> Result<Option<Frame>, SoftTncError> {
        // yes we want to deal with Frames here
        // it's important to establish successful decode that SoftDemodulator is aware of the frame innards
        Ok(None)
    }

    pub fn read_kiss(&mut self, _buf: &mut [u8]) -> Result<usize, SoftTncError> {
        Ok(0)
    }

    pub fn write_kiss(&mut self, _buf: &[u8]) -> Result<usize, SoftTncError> {
        Ok(0)
    }
}

#[derive(Debug)]
pub enum SoftTncError {
    General(&'static str),
}
