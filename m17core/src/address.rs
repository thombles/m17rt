#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Address {
    Invalid,
    Callsign(Callsign),
    Reserved(u64),
    Broadcast,
}

/// ASCII representation of a callsign address.
///
/// May be up to 9 characters long - if it shorter then remaining space is filled with
/// space characters.
///
/// If the "std" feature is enabled then callsigns be converted to or created from strings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Callsign([u8; 9]);

static ALPHABET: [u8; 40] = [
    b' ', b'A', b'B', b'C', b'D', b'E', b'F', b'G', b'H', b'I', b'J', b'K', b'L', b'M', b'N', b'O',
    b'P', b'Q', b'R', b'S', b'T', b'U', b'V', b'W', b'X', b'Y', b'Z', b'0', b'1', b'2', b'3', b'4',
    b'5', b'6', b'7', b'8', b'9', b'-', b'/', b'.',
];

// should be len 6
pub fn decode_address(encoded: [u8; 6]) -> Address {
    let full = u64::from_be_bytes([
        0, 0, encoded[0], encoded[1], encoded[2], encoded[3], encoded[4], encoded[5],
    ]);
    match full {
        m @ 1..=0xEE6B27FFFFFF => Address::Callsign(decode_base_40(m)),
        m @ 0xEE6B28000000..=0xFFFFFFFFFFFE => Address::Reserved(m),
        0xFFFFFFFFFFFF => Address::Broadcast,
        _ => Address::Invalid,
    }
}

fn decode_base_40(mut encoded: u64) -> Callsign {
    let mut callsign = Callsign([b' '; 9]);
    let mut pos = 0;
    while encoded > 0 {
        callsign.0[pos] = ALPHABET[(encoded % 40) as usize];
        encoded /= 40;
        pos += 1;
    }
    callsign
}

#[allow(dead_code)]
pub fn encode_address(address: &Address) -> [u8; 6] {
    let mut out: u64 = 0;
    match address {
        Address::Invalid => (),
        Address::Callsign(call) => {
            for c in call.0.iter().rev() {
                let c = c.to_ascii_uppercase();
                if let Some(pos) = ALPHABET.iter().position(|alpha| *alpha == c) {
                    out = out * 40 + pos as u64;
                }
            }
        }
        Address::Reserved(m) => out = *m,
        Address::Broadcast => out = 0xFFFFFFFFFFFF,
    }
    out.to_be_bytes()[2..].try_into().unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn address_encode() {
        let encoded = encode_address(&Address::Callsign(Callsign(
            b"AB1CD    ".as_slice().try_into().unwrap(),
        )));
        assert_eq!(encoded, [0x00, 0x00, 0x00, 0x9f, 0xdd, 0x51]);
    }

    #[test]
    fn address_decode() {
        let decoded = decode_address([0x00, 0x00, 0x00, 0x9f, 0xdd, 0x51]);
        assert_eq!(
            decoded,
            Address::Callsign(Callsign(b"AB1CD    ".as_slice().try_into().unwrap()))
        );
    }
}
