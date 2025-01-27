use crate::protocol::StreamFrame;

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
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct KissFrame {
    pub data: [u8; MAX_FRAME_LEN],
    pub len: usize,
}

impl KissFrame {
    /// Construct empty frame
    pub fn new_empty() -> Self {
        Self {
            data: [0u8; MAX_FRAME_LEN],
            len: 0,
        }
    }

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
    pub fn new_stream_data(frame: &StreamFrame) -> Result<Self, KissError> {
        let mut data = [0u8; MAX_FRAME_LEN];
        let mut i = 0;
        push(&mut data, &mut i, FEND);
        push(
            &mut data,
            &mut i,
            kiss_header(PORT_STREAM, KissCommand::DataFrame.proto_value()),
        );

        // 5 bytes LICH content
        i += escape(&frame.lich_part, &mut data[i..]);
        // 1 byte LICH metadata
        i += escape(&[frame.lich_idx << 5], &mut data[i..]);

        // 2 bytes frame number/EOS + 16 bytes payload + 2 bytes CRC
        let mut inner_data = [0u8; 20];
        let frame_num = frame.frame_number.to_be_bytes();
        inner_data[0] = frame_num[0] | if frame.end_of_stream { 0x80 } else { 0 };
        inner_data[1] = frame_num[1];
        inner_data[2..18].copy_from_slice(&frame.stream_data);
        let crc = crate::crc::m17_crc(&inner_data[0..18]);
        let crc_be = crc.to_be_bytes();
        inner_data[18] = crc_be[0];
        inner_data[19] = crc_be[1];
        i += escape(&inner_data, &mut data[i..]);

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
            .nth(1)
            .ok_or(KissError::MalformedKissFrame)?
            .0;
        let end = self.data[start..]
            .iter()
            .enumerate()
            .find(|(_, b)| **b == FEND)
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
        self.data
            .iter()
            .find(|b| **b != FEND)
            .cloned()
            .ok_or(KissError::MalformedKissFrame)
    }
}

fn kiss_header(port: u8, command: u8) -> u8 {
    (port << 4) | (command & 0x0f)
}

fn push(data: &mut [u8], idx: &mut usize, value: u8) {
    data[*idx] = value;
    *idx += 1;
}

#[derive(Debug, PartialEq, Eq, Clone)]
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

/// Accepts raw KISS data and emits one frame at a time.
///
/// A frame will be emitted if there is at least one byte between FEND markers. It is up to the consumer
/// to determine whether it's actually a valid frame.
pub struct KissBuffer {
    /// Provisional frame, whose buffer might contain more than one sequential frame at a time
    frame: KissFrame,
    /// Number of bytes that have been written into `frame.data`, which may be more than the the length
    /// of the first valid frame, `frame.len`.
    written: usize,
    /// Whether we have emitted the first frame in `frame`'s buffer and now need to flush it out.
    first_frame_returned: bool,
}

impl KissBuffer {
    /// Create new buffer
    pub fn new() -> Self {
        Self {
            frame: KissFrame::new_empty(),
            written: 0,
            first_frame_returned: false,
        }
    }

    /// Return the space remaining for more data
    pub fn buf_remaining(&mut self) -> &mut [u8] {
        self.flush_first_frame();
        if self.written == self.frame.data.len() {
            // full buffer with no data means oversized frame
            // sender is doing something weird or a FEND got dropped
            // either way: flush it all and try to sync up again
            self.written = 0;
        }
        &mut self.frame.data[self.written..]
    }

    /// Indicate how much data was written into the buffer provided by `buf_remaining()`.
    pub fn did_write(&mut self, len: usize) {
        self.written += len;
    }

    /// Try to construct and retrieve the next frame in the buffer
    pub fn next_frame(&mut self) -> Option<&KissFrame> {
        self.flush_first_frame();

        // If we have any data without a leading FEND, scan past it
        let mut i = 0;
        while i < self.written && self.frame.data[i] != FEND {
            i += 1;
        }
        self.move_to_start(i);

        // If we do have a leading FEND, scan up up to the last one in the series
        i = 0;
        while (i + 1) < self.written && self.frame.data[i + 1] == FEND {
            i += 1;
        }
        if i != 0 {
            self.move_to_start(i);
        }

        // Now, if we have FEND-something-FEND, return it
        if self.written >= 2 && self.frame.data[0] == FEND && self.frame.data[1] != FEND {
            i = 2;
            while i < self.written && self.frame.data[i] != FEND {
                i += 1;
            }
            if i < self.written && self.frame.data[i] == FEND {
                self.frame.len = i + 1;
                self.first_frame_returned = true;
                return Some(&self.frame);
            }
        }

        None
    }

    /// Check if we just returned a frame; if so, clear it out and position the buffer for the next frame.
    fn flush_first_frame(&mut self) {
        if !self.first_frame_returned {
            return;
        }
        self.first_frame_returned = false;
        // If we have previously returned a valid frame, in the simplest case `frame.data` contains FEND-something-FEND
        // So to find the trailing FEND we can start at index 2
        // Loop forward until we find that FEND, which must exist, and leave its index in `i`
        let mut i = 2;
        while self.frame.data[i] != FEND {
            i += 1;
        }
        // However if we have consecutive trailing FENDs we want to ignore them
        // Having found the trailing FEND, increment past any additional FENDs until we reach the end or something else
        while (i + 1) < self.written && self.frame.data[i + 1] == FEND {
            i += 1;
        }
        // Now take that final FEND and make it the start of our frame
        self.move_to_start(i);
    }

    /// Shift all data in the buffer back to the beginning starting from the given index.
    fn move_to_start(&mut self, idx: usize) {
        for i in idx..self.written {
            self.frame.data[i - idx] = self.frame.data[i];
        }
        self.written -= idx;
    }
}

impl Default for KissBuffer {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum KissError {
    MalformedKissFrame,
    UnsupportedKissCommand,
    PayloadTooBig,
    LsfWrongSize,
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

    #[test]
    fn test_buffer_basic() {
        let mut buffer = KissBuffer::new();

        // initial write is not a complete frame
        let buf = buffer.buf_remaining();
        buf[0] = FEND;
        buffer.did_write(1);
        assert!(buffer.next_frame().is_none());

        // complete the frame
        let buf = buffer.buf_remaining();
        buf[0] = 0x10;
        buf[1] = 0x01;
        buf[2] = FEND;
        buffer.did_write(3);

        // everything should parse
        let next = buffer.next_frame().unwrap();
        assert_eq!(next.len, 4);
        assert_eq!(&next.data[0..4], &[FEND, 0x10, 0x01, FEND]);
        assert_eq!(next.port().unwrap(), 1);
        assert_eq!(next.command().unwrap(), KissCommand::DataFrame);
        let mut payload_buf = [0u8; 1024];
        let n = next.decode_payload(&mut payload_buf).unwrap();
        assert_eq!(n, 1);
        assert_eq!(&payload_buf[0..n], &[0x01]);
    }

    #[test]
    fn test_buffer_double() {
        let mut buffer = KissBuffer::new();
        let buf = buffer.buf_remaining();
        buf[0..8].copy_from_slice(&[FEND, 0x10, 0x01, FEND, FEND, 0x20, 0x02, FEND]);
        buffer.did_write(8);

        let next = buffer.next_frame().unwrap();
        assert_eq!(next.port().unwrap(), 1);
        let next = buffer.next_frame().unwrap();
        assert_eq!(next.port().unwrap(), 2);
        assert!(buffer.next_frame().is_none());
    }

    #[test]
    fn test_buffer_double_shared_fend() {
        let mut buffer = KissBuffer::new();
        let buf = buffer.buf_remaining();
        buf[0..7].copy_from_slice(&[FEND, 0x10, 0x01, FEND, 0x20, 0x02, FEND]);
        buffer.did_write(7);

        let next = buffer.next_frame().unwrap();
        assert_eq!(next.port().unwrap(), 1);
        let next = buffer.next_frame().unwrap();
        assert_eq!(next.port().unwrap(), 2);
        assert!(buffer.next_frame().is_none());
    }

    #[test]
    fn test_buffer_extra_fend() {
        let mut buffer = KissBuffer::new();
        let buf = buffer.buf_remaining();
        buf[0..10].copy_from_slice(&[FEND, FEND, FEND, 0x10, 0x01, FEND, FEND, 0x20, 0x02, FEND]);
        buffer.did_write(10);

        let next = buffer.next_frame().unwrap();
        assert_eq!(next.port().unwrap(), 1);
        let next = buffer.next_frame().unwrap();
        assert_eq!(next.port().unwrap(), 2);
        assert!(buffer.next_frame().is_none());
    }

    #[test]
    fn test_buffer_oversize_frame() {
        let mut buffer = KissBuffer::new();
        let buf = buffer.buf_remaining();
        buf[0] = FEND;
        let len = buf.len();
        assert_eq!(len, MAX_FRAME_LEN);
        buffer.did_write(len);
        assert!(buffer.next_frame().is_none());

        let buf = buffer.buf_remaining();
        let len = buf.len();
        assert_eq!(len, MAX_FRAME_LEN); // should have flushed
        for i in 0..len / 2 {
            buf[i] = 0x00;
        }
        buffer.did_write(len / 2);
        assert!(buffer.next_frame().is_none());

        // confirm we resync if input goes back to normal
        let buf = buffer.buf_remaining();
        buf[0..4].copy_from_slice(&[FEND, 0x10, 0x01, FEND]);
        buffer.did_write(4);
        let next = buffer.next_frame().unwrap();
        assert_eq!(next.port().unwrap(), 1);
        assert!(buffer.next_frame().is_none());
    }
}
