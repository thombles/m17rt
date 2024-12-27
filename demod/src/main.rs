use log::debug;
use m17core::{
    modem::{Demodulator, SoftDemodulator},
    protocol::{Frame, LichCollection},
};
pub(crate) use std::{fs::File, io::Read};

pub fn run_my_decode() {
    let file = File::open("../../Data/test_vk7xt.rrc").unwrap();
    let mut input = file;
    let mut baseband = vec![];
    input.read_to_end(&mut baseband).unwrap();

    let mut lich = LichCollection::new();
    let mut codec2_data = vec![];
    let mut modem = SoftDemodulator::new();

    for pair in baseband.chunks(2) {
        let sample: i16 = i16::from_le_bytes([pair[0], pair[1]]);
        if let Some(frame) = modem.demod(sample) {
            debug!("Modem demodulated frame: {:?}", frame);
            if let Frame::Stream(s) = frame {
                for b in s.stream_data {
                    codec2_data.push(b);

                    let valid_before = lich.valid_segments();
                    lich.set_segment(s.lich_idx, s.lich_part);
                    let valid_after = lich.valid_segments();
                    if valid_before != valid_after {
                        debug!("Valid lich segments: {}", lich.valid_segments());
                    }
                    if valid_before == 5 && valid_after == 6 {
                        if let Some(l) = lich.try_assemble() {
                            debug!("Assembled complete lich: {l:?}");
                        }
                    }
                }
                if s.end_of_stream {
                    debug!("len of codec2 data: {}", codec2_data.len());
                    assert_eq!(codec2_data.len(), 1504);

                    m17codec2::decode_codec2(&codec2_data, "../../Data/speech_out.raw");
                }
            }
        }
    }
}

fn main() {
    env_logger::init();
    run_my_decode();
}
