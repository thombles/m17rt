// Note FEND and FESC both have the top two bits set. In the header byte this corresponds
// to high port numbers which are never used by M17 so we needn't bother (un)escaping it.

const FEND: u8 = 0xC0;
const FESC: u8 = 0xDB;
const TFEND: u8 = 0xDC;
const TFESC: u8 = 0xDD;

pub const PORT_PACKET_BASIC: u8 = 0;
pub const PORT_PACKET_FULL: u8 = 1;
pub const PORT_STREAM: u8 = 2;

/// Maximum theoretical frame size for any valid M17 KISS frame.
///
/// In M17 Full Packet Mode a 30-byte LSF is merged with a packet which may be up to
/// 825 bytes in length. Supposing an (impossible) worst case that every byte is FEND
/// or FESC, 1710 bytes is the maximum expected payload. With a FEND at each end and
/// the KISS frame header byte we get 1713.
pub const MAX_FRAME_LEN: usize = 1713;

/// Holder for any valid M17 KISS frame.
///
/// For efficiency, `data` and `len` are exposed directly and received KISS data may
/// be streamed directly into a pre-allocated `KissFrame`.
pub struct KissFrame {
    pub data: [u8; MAX_FRAME_LEN],
    pub len: usize,
}

impl KissFrame {
    /// Request to transmit a data packet (basic mode).
    ///
    /// A raw payload up to 822 bytes can be provided. The TNC will mark it as Raw format
    /// and automatically calculate the checksum. If required it will also chunk the packet
    /// into individual frames and transmit them sequentially.
    pub fn new_basic_packet(payload: &[u8]) -> Result<Self, KissError> {
        // M17 packet payloads can be up to 825 bytes in length
        // Type prefix (RAW = 0x00) occupies the first byte
        // Last 2 bytes are checksum
        if payload.len() > 822 {
            return Err(KissError::PayloadTooBig);
        }
        let mut data = [0u8; MAX_FRAME_LEN];
        let mut i = 0;
        push(&mut data, &mut i, FEND);
        push(
            &mut data,
            &mut i,
            kiss_header(PORT_PACKET_BASIC, KissCommand::DataFrame.proto_value()),
        );

        i += escape(payload, &mut data[i..]);
        push(&mut data, &mut i, FEND);

        Ok(KissFrame { data, len: i })
    }

    /// Request to transmit a data packet (full mode).
    ///
    /// Sender must provide a 30-byte LSF and a full packet payload (up to 825 bytes)
    /// that will be combined into the frame. The packet payload must include the type
    /// code prefix and the CRC, both of which would have been calculated by the TNC if
    /// it was basic mode.
    pub fn new_full_packet(lsf: &[u8], packet: &[u8]) -> Result<Self, KissError> {
        if lsf.len() != 30 {
            return Err(KissError::LsfWrongSize);
        }
        if packet.len() > 825 {
            return Err(KissError::PayloadTooBig);
        }
        let mut data = [0u8; MAX_FRAME_LEN];
        let mut i = 0;
        push(&mut data, &mut i, FEND);
        push(
            &mut data,
            &mut i,
            kiss_header(PORT_PACKET_FULL, KissCommand::DataFrame.proto_value()),
        );
        i += escape(lsf, &mut data[i..]);
        i += escape(packet, &mut data[i..]);
        push(&mut data, &mut i, FEND);

        Ok(KissFrame { data, len: i })
    }

    /// Request to begin a stream data transfer (e.g. voice).
    ///
    /// An LSF payload of exactly 30 bytes must be provided.
    ///
    /// This must be followed by at least one stream data payload, ending with one that
    /// has the end of stream (EOS) bit set.
    pub fn new_stream_setup(lsf: &[u8]) -> Result<Self, KissError> {
        if lsf.len() != 30 {
            return Err(KissError::LsfWrongSize);
        }
        let mut data = [0u8; MAX_FRAME_LEN];
        let mut i = 0;
        push(&mut data, &mut i, FEND);
        push(
            &mut data,
            &mut i,
            kiss_header(PORT_STREAM, KissCommand::DataFrame.proto_value()),
        );
        i += escape(lsf, &mut data[i..]);
        push(&mut data, &mut i, FEND);

        Ok(KissFrame { data, len: i })
    }

    /// Transmit a segment of data in a stream transfer (e.g. voice).
    ///
    /// A data payload of 26 bytes including metadata must be provided. This must follow
    /// exactly the prescribed format (H.5.2 in the spec). The TNC will be watching for
    /// the EOS flag to know that this transmission has ended.
    pub fn new_stream_data(stream_data: &[u8]) -> Result<Self, KissError> {
        if stream_data.len() != 26 {
            return Err(KissError::StreamDataWrongSize);
        }
        let mut data = [0u8; MAX_FRAME_LEN];
        let mut i = 0;
        push(&mut data, &mut i, FEND);
        push(
            &mut data,
            &mut i,
            kiss_header(PORT_STREAM, KissCommand::DataFrame.proto_value()),
        );
        i += escape(stream_data, &mut data[i..]);
        push(&mut data, &mut i, FEND);

        Ok(KissFrame { data, len: i })
    }

    /// Request to set the TxDelay
    pub fn new_set_tx_delay(port: u8, units: u8) -> Self {
        let mut data = [0u8; MAX_FRAME_LEN];
        let mut i = 0;
        push(&mut data, &mut i, FEND);
        push(
            &mut data,
            &mut i,
            kiss_header(port, KissCommand::TxDelay.proto_value()),
        );
        push(&mut data, &mut i, units);
        push(&mut data, &mut i, FEND);

        KissFrame { data, len: i }
    }

    /// Request to set the persistence parameter P
    pub fn new_set_p(port: u8, units: u8) -> Self {
        let mut data = [0u8; MAX_FRAME_LEN];
        let mut i = 0;
        push(&mut data, &mut i, FEND);
        push(
            &mut data,
            &mut i,
            kiss_header(port, KissCommand::P.proto_value()),
        );
        push(&mut data, &mut i, units);
        push(&mut data, &mut i, FEND);

        KissFrame { data, len: i }
    }

    /// Request to set full duplex or not
    pub fn set_full_duplex(port: u8, full_duplex: bool) -> Self {
        let mut data = [0u8; MAX_FRAME_LEN];
        let mut i = 0;
        push(&mut data, &mut i, FEND);
        push(
            &mut data,
            &mut i,
            kiss_header(port, KissCommand::FullDuplex.proto_value()),
        );
        push(&mut data, &mut i, if full_duplex { 1 } else { 0 });
        push(&mut data, &mut i, FEND);

        KissFrame { data, len: i }
    }

    /// Return this frame's KISS command type.
    pub fn command(&self) -> Result<KissCommand, KissError> {
        KissCommand::from_proto(self.header_byte()? & 0x0f)
    }

    /// Return the KISS port to which this frame relates. Should be 0, 1 or 2.
    pub fn port(&self) -> Result<u8, KissError> {
        Ok(self.header_byte()? >> 4)
    }

    /// Payload part of the frame between the header byte and the trailing FEND, unescaped.
    pub fn decode_payload(&self, out: &mut [u8]) -> Result<usize, KissError> {
        let start = self
            .data
            .iter()
            .enumerate()
            .skip_while(|(_, b)| **b == FEND)
            .skip(1)
            .next()
            .ok_or(KissError::MalformedKissFrame)?
            .0;
        let end = self.data[start..]
            .iter()
            .enumerate()
            .skip_while(|(_, b)| **b != FEND)
            .next()
            .ok_or(KissError::MalformedKissFrame)?
            .0
            + start;
        Ok(unescape(&self.data[start..end], out))
    }

    /// Borrow the frame as a slice
    pub fn as_bytes(&self) -> &[u8] {
        &self.data[..self.len]
    }

    /// Return the header byte of the KISS frame, skipping over 0 or more prepended FENDs.
    fn header_byte(&self) -> Result<u8, KissError> {
        Ok(self
            .data
            .iter()
            .skip_while(|b| **b == FEND)
            .next()
            .cloned()
            .ok_or(KissError::MalformedKissFrame)?)
    }
}

fn kiss_header(port: u8, command: u8) -> u8 {
    (port << 4) | (command & 0x0f)
}

fn push(data: &mut [u8], idx: &mut usize, value: u8) {
    data[*idx] = value;
    *idx += 1;
}

pub enum KissCommand {
    DataFrame,
    TxDelay,
    P,
    FullDuplex,
}

impl KissCommand {
    fn from_proto(value: u8) -> Result<Self, KissError> {
        Ok(match value {
            0 => KissCommand::DataFrame,
            1 => KissCommand::TxDelay,
            2 => KissCommand::P,
            5 => KissCommand::FullDuplex,
            _ => return Err(KissError::UnsupportedKissCommand),
        })
    }

    fn proto_value(&self) -> u8 {
        match self {
            KissCommand::DataFrame => 0,
            KissCommand::TxDelay => 1,
            KissCommand::P => 2,
            KissCommand::FullDuplex => 5,
        }
    }
}

#[derive(Debug)]
pub enum KissError {
    MalformedKissFrame,
    UnsupportedKissCommand,
    PayloadTooBig,
    LsfWrongSize,
    StreamDataWrongSize,
}

fn escape(src: &[u8], dst: &mut [u8]) -> usize {
    let mut i = 0;
    let mut j = 0;
    while i < src.len() && j < dst.len() {
        if src[i] == FEND {
            dst[j] = FESC;
            j += 1;
            dst[j] = TFEND;
        } else if src[i] == FESC {
            dst[j] = FESC;
            j += 1;
            dst[j] = TFESC;
        } else {
            dst[j] = src[i];
        }
        i += 1;
        j += 1;
    }
    j
}

fn unescape(src: &[u8], dst: &mut [u8]) -> usize {
    let mut i = 0;
    let mut j = 0;
    while i < src.len() && j < dst.len() {
        if src[i] == FESC {
            if i == src.len() - 1 {
                break;
            }
            i += 1;
            if src[i] == TFEND {
                dst[j] = FEND;
            } else if src[i] == TFESC {
                dst[j] = FESC;
            }
        } else {
            dst[j] = src[i];
        }
        i += 1;
        j += 1;
    }
    j
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_escape() {
        let mut buf = [0u8; 1024];

        let src = [0, 1, 2, 3, 4, 5];
        let n = escape(&src, &mut buf);
        assert_eq!(n, 6);
        assert_eq!(&buf[0..6], src);

        let src = [0, 1, TFESC, 3, TFEND, 5];
        let n = escape(&src, &mut buf);
        assert_eq!(n, 6);
        assert_eq!(&buf[0..6], src);

        let src = [0, 1, FEND, 3, 4, 5];
        let n = escape(&src, &mut buf);
        assert_eq!(n, 7);
        assert_eq!(&buf[0..7], &[0, 1, FESC, TFEND, 3, 4, 5]);

        let src = [0, 1, 2, 3, 4, FESC];
        let n = escape(&src, &mut buf);
        assert_eq!(n, 7);
        assert_eq!(&buf[0..7], &[0, 1, 2, 3, 4, FESC, TFESC]);
    }

    #[test]
    fn test_unescape() {
        let mut buf = [0u8; 1024];

        let src = [0, 1, 2, 3, 4, 5];
        let n = unescape(&src, &mut buf);
        assert_eq!(n, 6);
        assert_eq!(&buf[0..6], src);

        let src = [0, 1, TFESC, 3, TFEND, 5];
        let n = unescape(&src, &mut buf);
        assert_eq!(n, 6);
        assert_eq!(&buf[0..6], src);

        let src = [0, 1, FESC, TFEND, 3, 4, 5];
        let n = unescape(&src, &mut buf);
        assert_eq!(n, 6);
        assert_eq!(&buf[0..6], &[0, 1, FEND, 3, 4, 5]);

        let src = [0, 1, 2, 3, 4, FESC, TFESC];
        let n = unescape(&src, &mut buf);
        assert_eq!(n, 6);
        assert_eq!(&buf[0..6], &[0, 1, 2, 3, 4, FESC]);
    }

    #[test]
    fn basic_packet_roundtrip() {
        let f = KissFrame::new_basic_packet(&[0, 1, 2, 3]).unwrap();
        assert_eq!(f.as_bytes(), &[FEND, 0, 0, 1, 2, 3, FEND]);
        let mut buf = [0u8; 1024];
        let n = f.decode_payload(&mut buf).unwrap();
        assert_eq!(&buf[..n], &[0, 1, 2, 3]);
    }
}
