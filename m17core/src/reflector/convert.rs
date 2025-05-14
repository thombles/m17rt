//! Utilities for converting streams between UDP and RF representations

use crate::protocol::{LsfFrame, StreamFrame};

use super::packet::Voice;

/// Accepts `Voice` packets from a reflector and turns them into LSF and Stream frames.
///
/// This is the format required for the voice data to cross the KISS protocol boundary.
pub struct VoiceToRf {
    /// Link Setup most recently acquired
    lsf: Option<LsfFrame>,
    /// Which LICH part we are going to emit next, 0-5
    lich_cnt: usize,
}

impl VoiceToRf {
    pub fn new() -> Self {
        Self {
            lsf: None,
            lich_cnt: 0,
        }
    }

    /// For a Voice packet received from a reflector, return the frames that would be transmitted
    /// on RF, including by reconstructing the LICH parts of the stream frame.
    ///
    /// If this is the start of a new or different stream transmission, this returns the Link Setup
    /// Frame which comes first, then the first associated Stream frame.
    ///
    /// If this is a continuation of a transmission matching the previous LSF, then it returns only
    /// the Stream frame.
    pub fn next(&mut self, voice: &Voice) -> (Option<LsfFrame>, StreamFrame) {
        let this_lsf = voice.link_setup_frame();
        let emit_lsf = if Some(&this_lsf) != self.lsf.as_ref() {
            self.lsf = Some(this_lsf.clone());
            self.lich_cnt = 0;
            true
        } else {
            false
        };
        let lsf = self.lsf.as_ref().unwrap();
        let stream = StreamFrame {
            lich_idx: self.lich_cnt as u8,
            lich_part: (&lsf.0[self.lich_cnt * 5..(self.lich_cnt + 1) * 5])
                .try_into()
                .unwrap(),
            frame_number: voice.frame_number(),
            end_of_stream: voice.is_end_of_stream(),
            stream_data: voice.payload().try_into().unwrap(),
        };
        let lsf = if emit_lsf { self.lsf.clone() } else { None };
        if voice.is_end_of_stream() {
            self.lsf = None;
        }
        (lsf, stream)
    }
}
