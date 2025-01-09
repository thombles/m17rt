use crate::{
    bits::Bits,
    fec::{self, p_1},
    interleave::interleave,
    protocol::{LsfFrame, PacketFrame, StreamFrame, LSF_SYNC},
    random::random_xor,
};

pub(crate) fn encode_lsf(frame: &LsfFrame) -> [f32; 192] {
    let type3 = fec::encode(&frame.0, 240, p_1);
    let mut interleaved = interleave(&type3);
    random_xor(&mut interleaved);
    let mut out = [0f32; 192];
    for (val, o) in LSF_SYNC.iter().zip(out.iter_mut()) {
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

/*pub(crate) fn encode_stream(frame: &StreamFrame) -> [f32; 192] {
    let type3 = fec::encode(&frame.0, 240, p_1);
    let mut interleaved = interleave(&type3);
    random_xor(&mut interleaved);
    let mut out = [0f32; 192];
    for (val, o) in LSF_SYNC.iter().zip(out.iter_mut()) {
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
}*/

/*pub(crate) fn encode_packet(frame: &PacketFrame) -> [f32; 192] {
    let type3 = fec::encode(&frame.0, 240, p_1);
    let mut interleaved = interleave(&type3);
    random_xor(&mut interleaved);
    let mut out = [0f32; 192];
    for (val, o) in LSF_SYNC.iter().zip(out.iter_mut()) {
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
}*/

pub(crate) fn encode_lich(counter: u8, part: [u8; 5]) -> [u8; 12] {
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
        assert_eq!(decoded, Some(lsf));
    }

    #[test]
    fn lich_encode() {
        let input = [221, 81, 5, 5, 0];
        let counter = 2;
        let expected_output = [221, 82, 162, 16, 85, 200, 5, 14, 254, 4, 13, 153];
        assert_eq!(encode_lich(counter, input), expected_output);
    }

    #[test]
    fn lich_round_trip() {
        let input = [1, 255, 0, 90, 10];
        let counter = 0;
        assert_eq!(
            crate::decode::decode_lich(&encode_lich(counter, input.clone())),
            Some((counter, input))
        );
    }
}
