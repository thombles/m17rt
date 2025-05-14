pub const M17_ALG: crc::Algorithm<u16> = crc::Algorithm {
    width: 16,
    poly: 0x5935,
    init: 0xFFFF,
    refin: false,
    refout: false,
    xorout: 0x0000,
    check: 0x772B,
    residue: 0x0000,
};

pub fn m17_crc(input: &[u8]) -> u16 {
    let crc = crc::Crc::<u16>::new(&M17_ALG);
    let mut digest = crc.digest();
    digest.update(input);
    digest.finalize()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc_test_vectors() {
        assert_eq!(m17_crc(&[]), 0xFFFF);
        assert_eq!(m17_crc("A".as_bytes()), 0x206E);
        assert_eq!(m17_crc("123456789".as_bytes()), 0x772B);
        let bytes: Vec<u8> = (0x00..=0xFF).collect();
        assert_eq!(m17_crc(&bytes), 0x1C31);
    }
}
