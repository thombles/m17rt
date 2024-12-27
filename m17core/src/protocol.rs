use crate::address::Address;

pub(crate) const LSF_SYNC: [i8; 8] = [1, 1, 1, 1, -1, -1, 1, -1];
pub(crate) const BERT_SYNC: [i8; 8] = [-1, 1, -1, -1, 1, 1, 1, 1];
pub(crate) const STREAM_SYNC: [i8; 8] = [-1, -1, -1, -1, 1, 1, -1, 1];
pub(crate) const PACKET_SYNC: [i8; 8] = [1, -1, 1, 1, -1, -1, -1, -1];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mode {
    Packet,
    Stream,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DataType {
    Reserved,
    Data,
    Voice,
    VoiceAndData,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EncryptionType {
    None,
    Scrambler,
    Aes,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Frame {
    Lsf(LsfFrame),
    Stream(StreamFrame),
    // Packet
    // BERT
}

pub enum PacketType {
    /// RAW
    Raw,
    /// AX.25
    Ax25,
    /// APRS
    Aprs,
    /// 6LoWPAN
    SixLowPan,
    /// IPv4
    Ipv4,
    /// SMS
    Sms,
    /// Winlink
    Winlink,
    /// Custom identifier
    Other(char),
}

impl PacketType {
    pub fn as_proto(&self) -> ([u8; 4], usize) {
        match self {
            PacketType::Raw => ([0, 0, 0, 0], 1),
            PacketType::Ax25 => ([1, 0, 0, 0], 1),
            PacketType::Aprs => ([2, 0, 0, 0], 1),
            PacketType::SixLowPan => ([3, 0, 0, 0], 1),
            PacketType::Ipv4 => ([4, 0, 0, 0], 1),
            PacketType::Sms => ([5, 0, 0, 0], 1),
            PacketType::Winlink => ([6, 0, 0, 0], 1),
            PacketType::Other(c) => {
                let mut buf = [0u8; 4];
                let s = c.encode_utf8(&mut buf);
                let len = s.len();
                (buf, len)
            }
        }
    }

    pub fn from_proto(&self, buf: &[u8]) -> Option<PacketType> {
        buf.utf8_chunks()
            .next()
            .and_then(|chunk| chunk.valid().chars().next())
            .map(|c| match c as u32 {
                0x00 => PacketType::Raw,
                0x01 => PacketType::Ax25,
                0x02 => PacketType::Aprs,
                0x03 => PacketType::SixLowPan,
                0x04 => PacketType::Ipv4,
                0x05 => PacketType::Sms,
                0x06 => PacketType::Winlink,
                _ => PacketType::Other(c),
            })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LsfFrame(pub [u8; 30]);

impl LsfFrame {
    pub fn crc(&self) -> u16 {
        crate::crc::m17_crc(&self.0)
    }

    pub fn destination(&self) -> Address {
        crate::address::decode_address((&self.0[0..6]).try_into().unwrap())
    }

    pub fn source(&self) -> Address {
        crate::address::decode_address((&self.0[6..12]).try_into().unwrap())
    }

    pub fn mode(&self) -> Mode {
        if self.0[12] & 0x01 > 0 {
            Mode::Stream
        } else {
            Mode::Packet
        }
    }

    pub fn data_type(&self) -> DataType {
        match (self.0[12] >> 1) & 0x03 {
            0b00 => DataType::Reserved,
            0b01 => DataType::Data,
            0b10 => DataType::Voice,
            0b11 => DataType::VoiceAndData,
            _ => unreachable!(),
        }
    }

    pub fn encryption_type(&self) -> EncryptionType {
        match (self.0[12] >> 3) & 0x03 {
            0b00 => EncryptionType::None,
            0b01 => EncryptionType::Scrambler,
            0b10 => EncryptionType::Aes,
            0b11 => EncryptionType::Other,
            _ => unreachable!(),
        }
    }

    pub fn channel_access_number(&self) -> u8 {
        (self.0[12] >> 7) & 0x0f
    }

    pub fn meta(&self) -> [u8; 14] {
        self.0[14..28].try_into().unwrap()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamFrame {
    /// Which LICH segment is given in this frame, from 0 to 5 inclusive
    pub lich_idx: u8,
    /// Decoded LICH segment
    pub lich_part: [u8; 5],
    /// Which frame in the transmission this is, starting from 0
    pub frame_number: u16,
    /// Is this the last frame in the transmission?
    pub end_of_stream: bool,
    /// Raw application data in this frame
    pub stream_data: [u8; 16],
}

pub struct LichCollection([Option<[u8; 5]>; 6]);

impl LichCollection {
    pub fn new() -> Self {
        Self([None; 6])
    }

    pub fn valid_segments(&self) -> usize {
        self.0.iter().filter(|s| s.is_some()).count()
    }

    pub fn set_segment(&mut self, counter: u8, part: [u8; 5]) {
        self.0[counter as usize] = Some(part);
    }

    pub fn try_assemble(&self) -> Option<[u8; 30]> {
        let mut out = [0u8; 30];
        for (i, segment) in self.0.iter().enumerate() {
            let Some(segment) = segment else {
                return None;
            };
            for (j, seg_val) in segment.iter().enumerate() {
                out[i * 5 + j] = *seg_val;
            }
        }
        Some(out)
    }
}

impl Default for LichCollection {
    fn default() -> Self {
        Self::new()
    }
}
