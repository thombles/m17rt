//! Utilities for converting streams between UDP and RF representations

use crate::protocol::{LsfFrame, StreamFrame};

use super::packet::Voice;

/// Accepts `Voice` packets from a reflector and turns them into LSF and Stream frames.
///
/// This is the format required for the voice data to cross the KISS protocol boundary.
#[derive(Debug, Default)]
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

/// Accepts LSF and stream RF payloads and merges them into `Voice` packets for reflector use.
///
/// For a series of transmissions this object should be re-used so that Stream ID is correctly
/// changed after each new LSF.
#[derive(Debug, Clone)]
pub struct RfToVoice {
    lsf: LsfFrame,
    stream_id: u16,
}

impl RfToVoice {
    pub fn new(lsf: LsfFrame) -> Self {
        // no_std "random"
        let stream_id = &lsf as *const LsfFrame as u16;
        Self { lsf, stream_id }
    }

    pub fn process_lsf(&mut self, lsf: LsfFrame) {
        self.lsf = lsf;
        self.stream_id = self.stream_id.wrapping_add(1);
    }

    pub fn process_stream(&self, stream: &StreamFrame) -> Voice {
        let mut v = Voice::new();
        v.set_stream_id(self.stream_id);
        v.set_frame_number(stream.frame_number);
        v.set_end_of_stream(stream.end_of_stream);
        v.set_payload(&stream.stream_data);
        v.set_link_setup_frame(&self.lsf);
        v
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        address::{Address, Callsign},
        protocol::{LsfFrame, StreamFrame},
    };

    use super::{RfToVoice, VoiceToRf};

    #[test]
    fn convert_roundtrip() {
        let lsf = LsfFrame::new_voice(
            &Address::Callsign(Callsign(*b"VK7XT    ")),
            &Address::Broadcast,
        );
        let stream = StreamFrame {
            lich_idx: 0,
            lich_part: lsf.0[0..5].try_into().unwrap(),
            frame_number: 0,
            end_of_stream: false,
            stream_data: [1u8; 16],
        };
        let rf_to_voice = RfToVoice::new(lsf.clone());
        let voice = rf_to_voice.process_stream(&stream);

        let mut voice_to_rf = VoiceToRf::new();
        let (lsf2, stream2) = voice_to_rf.next(&voice);
        assert_eq!(lsf2, Some(lsf));
        assert_eq!(stream2, stream);
    }
}
