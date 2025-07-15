use crate::{
    bits::Bits,
    fec::{self, p_1, p_2, p_3},
    interleave::interleave,
    protocol::{
        LSF_SYNC, LsfFrame, PACKET_SYNC, PacketFrame, PacketFrameCounter, STREAM_SYNC, StreamFrame,
    },
    random::random_xor,
};

pub(crate) fn encode_lsf(frame: &LsfFrame) -> [f32; 192] {
    let type3 = fec::encode(&frame.0, 240, p_1);
    interleave_to_dibits(type3, LSF_SYNC)
}

pub(crate) fn encode_stream(frame: &StreamFrame) -> [f32; 192] {
    let lich = encode_lich(frame.lich_idx, &frame.lich_part);
    let mut type1 = [0u8; 18];
    let frame_number = frame.frame_number | if frame.end_of_stream { 0x8000 } else { 0x0000 };
    type1[0..2].copy_from_slice(&frame_number.to_be_bytes());
    type1[2..18].copy_from_slice(&frame.stream_data);
    let type3 = fec::encode(&type1, 144, p_2);
    let mut combined = [0u8; 46];
    combined[0..12].copy_from_slice(&lich);
    combined[12..46].copy_from_slice(&type3[0..34]);
    interleave_to_dibits(combined, STREAM_SYNC)
}

pub(crate) fn encode_packet(frame: &PacketFrame) -> [f32; 192] {
    let mut type1 = [0u8; 26]; // only 206 out of 208 bits filled
    match frame.counter {
        PacketFrameCounter::Frame { index } => {
            type1[0..25].copy_from_slice(&frame.payload);
            type1[25] = (index as u8) << 2;
        }
        PacketFrameCounter::FinalFrame { payload_len } => {
            type1[0..payload_len].copy_from_slice(&frame.payload[0..payload_len]);
            type1[25] = ((payload_len as u8) << 2) | 0x80;
        }
    }
    let type3 = fec::encode(&type1, 206, p_3);
    interleave_to_dibits(type3, PACKET_SYNC)
}

/// Generate a preamble suitable for placement before an LSF frame.
///
/// Polarity needs to be flipped for BERT, however we don't support this yet.
/// STREAM and PACKET don't need to be considered as they are an invalid way to
/// begin a transmission.
pub(crate) fn generate_preamble() -> [f32; 192] {
    // TODO: should all these encode/generate functions return owning iterators?
    // Then I could avoid making this array which I'm just going to have to copy anyway
    let mut out = [1.0f32; 192];
    for n in out.iter_mut().skip(1).step_by(2) {
        *n = -1.0;
    }
    out
}

pub(crate) fn generate_end_of_transmission() -> [f32; 192] {
    let mut out = [1.0f32; 192];
    for n in out.iter_mut().skip(6).step_by(8) {
        *n = -1.0;
    }
    out
}

pub(crate) fn encode_lich(counter: u8, part: &[u8; 5]) -> [u8; 12] {
    let mut out = [0u8; 12];
    let to_encode = [
        ((part[0] as u16) << 4) | ((part[1] as u16) >> 4),
        ((part[1] as u16 & 0x000f) << 8) | part[2] as u16,
        ((part[3] as u16) << 4) | ((part[4] as u16) >> 4),
        ((part[4] as u16 & 0x000f) << 8) | ((counter as u16) << 5),
    ];
    for (i, o) in to_encode.into_iter().zip(out.chunks_mut(3)) {
        let encoded = cai_golay::extended::encode(i).to_be_bytes();
        o[0..3].copy_from_slice(&encoded[1..4]);
    }
    out
}

fn interleave_to_dibits(combined: [u8; 46], sync_burst: [i8; 8]) -> [f32; 192] {
    let mut interleaved = interleave(&combined);
    random_xor(&mut interleaved);
    let mut out = [0f32; 192];
    for (val, o) in sync_burst.iter().zip(out.iter_mut()) {
        *o = *val as f32;
    }
    let bits = Bits::new(&interleaved);
    let mut out_bits = bits.iter();
    for o in out[8..].iter_mut() {
        *o = match (out_bits.next().unwrap(), out_bits.next().unwrap()) {
            (0, 1) => 1.0,
            (0, 0) => 1.0 / 3.0,
            (1, 0) => -1.0 / 3.0,
            (1, 1) => -1.0,
            _ => unreachable!(),
        };
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lsf_round_trip() {
        let lsf = LsfFrame([
            255, 255, 255, 255, 255, 255, 0, 0, 0, 159, 221, 81, 5, 5, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 131, 53,
        ]);
        let encoded = encode_lsf(&lsf);
        let decoded = crate::decode::parse_lsf(&encoded);
        assert!(matches!(decoded, Some((frame, _)) if frame == lsf));
    }

    #[test]
    fn stream_round_trip() {
        let stream = StreamFrame {
            lich_idx: 5,
            lich_part: [1, 2, 3, 4, 5],
            frame_number: 50,
            end_of_stream: false,
            stream_data: [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16],
        };
        let encoded = encode_stream(&stream);
        let decoded = crate::decode::parse_stream(&encoded);
        assert!(matches!(decoded, Some((frame, _)) if frame == stream));
    }

    #[test]
    fn packet_round_trip() {
        let packet = PacketFrame {
            payload: [41u8; 25],
            counter: PacketFrameCounter::Frame { index: 3 },
        };
        let encoded = encode_packet(&packet);
        let decoded = crate::decode::parse_packet(&encoded);
        assert!(matches!(decoded, Some((frame, _)) if frame == packet));

        let packet = PacketFrame {
            payload: [0u8; 25],
            counter: PacketFrameCounter::FinalFrame { payload_len: 10 },
        };
        let encoded = encode_packet(&packet);
        let decoded = crate::decode::parse_packet(&encoded);
        assert!(matches!(decoded, Some((frame, _)) if frame == packet));
    }

    #[test]
    fn lich_encode() {
        let input = [221, 81, 5, 5, 0];
        let counter = 2;
        let expected_output = [221, 82, 162, 16, 85, 200, 5, 14, 254, 4, 13, 153];
        assert_eq!(encode_lich(counter, &input), expected_output);
    }

    #[test]
    fn lich_round_trip() {
        let input = [1, 255, 0, 90, 10];
        let counter = 0;
        assert_eq!(
            crate::decode::decode_lich(&encode_lich(counter, &input)),
            Some((counter, input))
        );
    }
}
