use crate::{
    bits::BitsMut,
    fec::{self, p_1, p_2, p_3},
    interleave::interleave,
    protocol::{
        LsfFrame, PacketFrame, PacketFrameCounter, StreamFrame, BERT_SYNC, LSF_SYNC, PACKET_SYNC,
        STREAM_SYNC,
    },
    random::random_xor,
};
use log::debug;

const PLUS_THREE: [u8; 2] = [0, 1];
const PLUS_ONE: [u8; 2] = [0, 0];
const MINUS_ONE: [u8; 2] = [1, 0];
const MINUS_THREE: [u8; 2] = [1, 1];

fn decode_sample(sample: f32) -> [u8; 2] {
    if sample > 0.667 {
        PLUS_THREE
    } else if sample > 0.0 {
        PLUS_ONE
    } else if sample > -0.667 {
        MINUS_ONE
    } else {
        MINUS_THREE
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SyncBurst {
    Lsf,
    Bert,
    Stream,
    Packet,
}

impl SyncBurst {
    pub(crate) fn target(&self) -> [i8; 8] {
        match self {
            Self::Lsf => LSF_SYNC,
            Self::Bert => BERT_SYNC,
            Self::Stream => STREAM_SYNC,
            Self::Packet => PACKET_SYNC,
        }
    }
}

const SYNC_MIN_GAIN: f32 = 16.0;
const SYNC_BIT_THRESHOLD: f32 = 0.3;
pub const SYNC_THRESHOLD: f32 = 100.0;

pub(crate) fn sync_burst_correlation(target: [i8; 8], samples: &[f32]) -> (f32, f32, f32) {
    let mut pos_max: f32 = f32::MIN;
    let mut neg_max: f32 = f32::MAX;
    for i in 0..8 {
        pos_max = pos_max.max(samples[i * 10]);
        neg_max = neg_max.min(samples[i * 10]);
    }
    let gain = (pos_max - neg_max) / 2.0;
    let shift = pos_max + neg_max;
    if gain < SYNC_MIN_GAIN {
        return (f32::MAX, gain, shift);
    }

    let mut diff = 0.0;
    for i in 0..8 {
        let sym_diff = (((samples[i * 10] - shift) / gain) - target[i] as f32).abs();
        if sym_diff > SYNC_BIT_THRESHOLD {
            return (f32::MAX, gain, shift);
        }
        diff += sym_diff;
    }
    (diff, gain, shift)
}

/// Decode frame and return contents after the sync burst
pub(crate) fn frame_initial_decode(frame: &[f32] /* length 192 */) -> [u8; 46] {
    let mut decoded = [0u8; 48];
    let mut decoded_bits = BitsMut::new(&mut decoded);
    for (idx, s) in frame.iter().enumerate() {
        let dibits = decode_sample(*s);
        decoded_bits.set_bit(idx * 2, dibits[0]);
        decoded_bits.set_bit(idx * 2 + 1, dibits[1]);
    }
    random_xor(&mut decoded[2..]);
    interleave(&decoded[2..])
}

pub(crate) fn parse_lsf(frame: &[f32] /* length 192 */) -> Option<LsfFrame> {
    let deinterleaved = frame_initial_decode(frame);
    debug!("deinterleaved: {:?}", deinterleaved);
    let lsf = match fec::decode(&deinterleaved, 240, p_1) {
        Some(lsf) => LsfFrame(lsf),
        None => return None,
    };
    debug!("full lsf: {:?}", lsf.0);
    let crc = lsf.check_crc();
    debug!("recv crc: {:04X}", crc);
    debug!("destination: {:?}", lsf.destination());
    debug!("source: {:?}", lsf.source());
    debug!("mode: {:?}", lsf.mode());
    debug!("data type: {:?}", lsf.data_type());
    debug!("encryption type: {:?}", lsf.encryption_type());
    debug!("can: {}", lsf.channel_access_number());
    debug!("meta: {:?}", lsf.meta());
    Some(lsf)
}

pub(crate) fn parse_stream(frame: &[f32] /* length 192 */) -> Option<StreamFrame> {
    let deinterleaved = frame_initial_decode(frame);
    let stream_part = &deinterleaved[12..];
    let stream = match fec::decode(stream_part, 144, p_2) {
        Some(stream) => stream,
        None => return None,
    };
    let frame_num = u16::from_be_bytes([stream[0], stream[1]]);
    let eos = (frame_num & 0x8000) > 0;
    let frame_num = frame_num & 0x7fff; // higher layer has to handle wraparound
    debug!("frame number: {frame_num}, codec2: {:?}", &stream[2..18]);

    if let Some((counter, part)) = decode_lich(&deinterleaved[0..12]) {
        debug!(
            "LICH: received part {counter} part {part:?} from raw {:?}",
            &deinterleaved[0..12]
        );
        Some(StreamFrame {
            lich_idx: counter,
            lich_part: part,
            frame_number: frame_num,
            end_of_stream: eos,
            stream_data: stream[2..18].try_into().unwrap(),
        })
    } else {
        None
    }
}

pub(crate) fn parse_packet(frame: &[f32] /* length 192 */) -> Option<PacketFrame> {
    let deinterleaved = frame_initial_decode(frame);
    let packet = match fec::decode(&deinterleaved, 206, p_3) {
        Some(packet) => packet,
        None => return None,
    };
    // TODO: the spec is inconsistent about which bit in packet[25] is EOF
    // https://github.com/M17-Project/M17_spec/issues/147
    let final_frame = (packet[25] & 0x04) > 0;
    let number = packet[25] >> 3;
    let counter = if final_frame {
        PacketFrameCounter::FinalFrame {
            payload_len: number as usize,
        }
    } else {
        PacketFrameCounter::Frame {
            index: number as usize,
        }
    };
    Some(PacketFrame {
        payload: packet[0..25].try_into().unwrap(),
        counter,
    })
}

pub(crate) fn decode_lich(type2_bits: &[u8]) -> Option<(u8, [u8; 5])> {
    let mut decoded = 0u64;
    for (input_idx, input_bytes) in type2_bits.chunks(3).enumerate() {
        let mut input: u32 = 0;
        for (idx, byte) in input_bytes.iter().enumerate() {
            input |= (*byte as u32) << (16 - (8 * idx));
        }
        let (val, _dist) = cai_golay::extended::decode(input)?;
        decoded |= (val as u64) << ((3 - input_idx) * 12);
    }
    let b = decoded.to_be_bytes();
    Some((b[7] >> 5, [b[2], b[3], b[4], b[5], b[6]]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lich_decode() {
        let input = [221, 82, 162, 16, 85, 200, 5, 14, 254, 4, 13, 153];
        let expected_counter = 2;
        let expected_part = [221, 81, 5, 5, 0];
        assert_eq!(decode_lich(&input), Some((expected_counter, expected_part)));
    }
}
