use crate::{
    address::{encode_address, Address},
    bits::BitsMut,
};

pub(crate) const LSF_SYNC: [i8; 8] = [1, 1, 1, 1, -1, -1, 1, -1];
pub(crate) const BERT_SYNC: [i8; 8] = [-1, 1, -1, -1, 1, 1, 1, 1];
pub(crate) const STREAM_SYNC: [i8; 8] = [-1, -1, -1, -1, 1, 1, -1, 1];
pub(crate) const PACKET_SYNC: [i8; 8] = [1, -1, 1, 1, -1, -1, -1, -1];
pub(crate) const PREAMBLE: [i8; 8] = [1, -1, 1, -1, 1, -1, 1, -1];
pub(crate) const END_OF_TRANSMISSION: [i8; 8] = [1, 1, 1, 1, 1, 1, -1, 1];

#[derive(Debug, Clone, PartialEq, Eq, Copy)]
pub enum Mode {
    Packet,
    Stream,
}
#[derive(Debug, Clone, PartialEq, Eq, Copy)]
pub enum DataType {
    Reserved,
    Data,
    Voice,
    VoiceAndData,
}
#[derive(Debug, Clone, PartialEq, Eq, Copy)]
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
    Packet(PacketFrame),
    // BERT
}

#[derive(Debug, Clone, PartialEq, Eq, Copy)]
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
    pub fn from_proto(buf: &[u8]) -> Option<(Self, usize)> {
        buf.utf8_chunks()
            .next()
            .and_then(|chunk| chunk.valid().chars().next())
            .map(|c| match c as u32 {
                0x00 => (PacketType::Raw, 1),
                0x01 => (PacketType::Ax25, 1),
                0x02 => (PacketType::Aprs, 1),
                0x03 => (PacketType::SixLowPan, 1),
                0x04 => (PacketType::Ipv4, 1),
                0x05 => (PacketType::Sms, 1),
                0x06 => (PacketType::Winlink, 1),
                _ => (PacketType::Other(c), c.len_utf8()),
            })
    }

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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LsfFrame(pub [u8; 30]);

impl LsfFrame {
    pub fn new_voice(source: &Address, destination: &Address) -> Self {
        let mut out = Self([0u8; 30]);
        out.set_source(source);
        out.set_destination(destination);
        out.set_mode(Mode::Stream);
        out.set_data_type(DataType::Voice);
        out.set_encryption_type(EncryptionType::None);
        out
    }

    pub fn new_packet(source: &Address, destination: &Address) -> Self {
        let mut out = Self([0u8; 30]);
        out.set_source(source);
        out.set_destination(destination);
        out.set_mode(Mode::Packet);
        out.set_data_type(DataType::Data);
        out.set_encryption_type(EncryptionType::None);
        out
    }

    /// Calculate crc of entire frame. If zero, it is a valid frame.
    pub fn check_crc(&self) -> u16 {
        crate::crc::m17_crc(&self.0)
    }

    pub fn destination(&self) -> Address {
        crate::address::decode_address((&self.0[0..6]).try_into().unwrap())
    }

    pub fn source(&self) -> Address {
        crate::address::decode_address((&self.0[6..12]).try_into().unwrap())
    }

    pub fn mode(&self) -> Mode {
        if self.lsf_type() & 0x0001 > 0 {
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
        match (self.lsf_type() >> 3) & 0x0003 {
            0b00 => EncryptionType::None,
            0b01 => EncryptionType::Scrambler,
            0b10 => EncryptionType::Aes,
            0b11 => EncryptionType::Other,
            _ => unreachable!(),
        }
    }

    // TODO: encryption sub-type

    pub fn channel_access_number(&self) -> u8 {
        ((self.lsf_type() >> 7) & 0x000f) as u8
    }

    pub fn meta(&self) -> [u8; 14] {
        self.0[14..28].try_into().unwrap()
    }

    pub fn set_destination(&mut self, destination: &Address) {
        self.0[0..6].copy_from_slice(&encode_address(destination));
        self.recalculate_crc();
    }

    pub fn set_source(&mut self, source: &Address) {
        self.0[6..12].copy_from_slice(&encode_address(source));
        self.recalculate_crc();
    }

    pub fn set_mode(&mut self, mode: Mode) {
        let existing_type = self.lsf_type();
        let new_type = (existing_type & !0x0001) | if mode == Mode::Stream { 1 } else { 0 };
        self.0[12..14].copy_from_slice(&new_type.to_be_bytes());
        self.recalculate_crc();
    }

    pub fn set_data_type(&mut self, data_type: DataType) {
        let type_part = match data_type {
            DataType::Reserved => 0b00 << 1,
            DataType::Data => 0b01 << 1,
            DataType::Voice => 0b10 << 1,
            DataType::VoiceAndData => 0b11 << 1,
        };
        let existing_type = self.lsf_type();
        let new_type = (existing_type & !0x0006) | type_part;
        self.0[12..14].copy_from_slice(&new_type.to_be_bytes());
        self.recalculate_crc();
    }

    pub fn set_encryption_type(&mut self, encryption_type: EncryptionType) {
        let type_part = match encryption_type {
            EncryptionType::None => 0b00 << 3,
            EncryptionType::Scrambler => 0b01 << 3,
            EncryptionType::Aes => 0b10 << 3,
            EncryptionType::Other => 0b11 << 3,
        };
        let existing_type = self.lsf_type();
        let new_type = (existing_type & !0x0018) | type_part;
        self.0[12..14].copy_from_slice(&new_type.to_be_bytes());
        self.recalculate_crc();
    }

    pub fn set_channel_access_number(&mut self, number: u8) {
        let mut bits = BitsMut::new(&mut self.0);
        bits.set_bit(12 * 8 + 5, (number >> 3) & 1);
        bits.set_bit(12 * 8 + 6, (number >> 2) & 1);
        bits.set_bit(12 * 8 + 7, (number >> 1) & 1);
        bits.set_bit(13 * 8, number & 1);
        self.recalculate_crc();
    }

    fn recalculate_crc(&mut self) {
        let new_crc = crate::crc::m17_crc(&self.0[0..28]);
        self.0[28..30].copy_from_slice(&new_crc.to_be_bytes());
        debug_assert_eq!(self.check_crc(), 0);
    }

    fn lsf_type(&self) -> u16 {
        u16::from_be_bytes([self.0[12], self.0[13]])
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PacketFrame {
    /// Application packet payload (chunk)
    pub payload: [u8; 25],

    /// Frame counter, which provides different information depending on whether this is the last frame or not.
    pub counter: PacketFrameCounter,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PacketFrameCounter {
    /// Any packet frame that comes after the LSF and is not the final frame.
    Frame {
        /// Which frame this is in the superframe, from 0 to 31 inclusive.
        ///
        /// If a 33rd frame exists (index 32), it will be a `FinalFrame` instead.
        ///
        /// All 25 bytes of of `payload` are filled and valid.
        index: usize,
    },
    /// The final frame in the packet superframe.
    FinalFrame {
        /// The number of bytes in `payload` that are filled.
        payload_len: usize,
    },
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_can() {
        let mut frame = LsfFrame([0u8; 30]);
        frame.set_channel_access_number(11);
        assert_eq!(frame.channel_access_number(), 11);
    }
}
