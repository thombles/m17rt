// Based on https://github.com/n7tae/mrefd/blob/master/Packet-Description.md
// and the main M17 specification

use crate::protocol::LsfFrame;

macro_rules! impl_stream_id {
    ($t:ty, $from:tt) => {
        impl $t {
            pub fn stream_id(&self) -> u16 {
                u16::from_be_bytes([self.0[$from], self.0[$from + 1]])
            }
        }
    };
}

macro_rules! impl_link_setup {
    ($t:ty, $from:tt) => {
        impl $t {
            pub fn link_setup_frame(&self) -> LsfFrame {
                let mut frame = LsfFrame([0; 30]);
                frame.0[0..28].copy_from_slice(&self.0[$from..($from + 28)]);
                frame.recalculate_crc();
                frame
            }
        }
    };
}

macro_rules! impl_link_setup_frame {
    ($t:ty, $from:tt) => {
        impl $t {
            pub fn link_setup_frame(&self) -> LsfFrame {
                let mut frame = LsfFrame([0; 30]);
                frame.0[..].copy_from_slice(&self.0[$from..($from + 30)]);
                frame
            }
        }
    };
}

macro_rules! impl_frame_number {
    ($t:ty, $from:tt) => {
        impl $t {
            pub fn frame_number(&self) -> u16 {
                let frame_num = u16::from_be_bytes([self.0[$from], self.0[$from + 1]]);
                frame_num & 0x7fff
            }

            pub fn is_end_of_stream(&self) -> bool {
                let frame_num = u16::from_be_bytes([self.0[$from], self.0[$from + 1]]);
                (frame_num & 0x8000) > 0
            }
        }
    };
}

macro_rules! impl_payload {
    ($t:ty, $from:tt, $to:tt) => {
        impl $t {
            pub fn payload(&self) -> &[u8] {
                &self.0[$from..$to]
            }
        }
    };
}

macro_rules! impl_modules {
    ($t:ty, $from:tt, $to:tt) => {
        impl $t {
            pub fn modules(&self) -> ModulesIterator {
                ModulesIterator::new(&self.0[$from..$to])
            }
        }
    };
}

macro_rules! impl_module {
    ($t:ty, $at:tt) => {
        impl $t {
            pub fn module(&self) -> char {
                self.0[$at] as char
            }
        }
    };
}

macro_rules! impl_address {
    ($t:ty, $from:tt) => {
        impl $t {
            pub fn address(&self) -> crate::address::Address {
                crate::address::decode_address(self.0[$from..($from + 6)].try_into().unwrap())
            }
        }
    };
}

macro_rules! impl_trailing_crc_verify {
    ($t:ty) => {
        impl $t {
            pub fn verify_integrity(&self) -> bool {
                crate::crc::m17_crc(&self.0) == 0
            }
        }
    };
}

macro_rules! impl_internal_crc {
    ($t:ty, $from:tt, $to:tt) => {
        impl $t {
            pub fn verify_integrity(&self) -> bool {
                crate::crc::m17_crc(&self.0[$from..$to]) == 0
            }
        }
    };
}

macro_rules! impl_is_relayed {
    ($t:ty) => {
        impl $t {
            pub fn is_relayed(&self) -> bool {
                self.0[self.0.len() - 1] != 0
            }
        }
    };
}

pub struct ModulesIterator<'a> {
    modules: &'a [u8],
    idx: usize,
}

impl<'a> ModulesIterator<'a> {
    fn new(modules: &'a [u8]) -> Self {
        Self { modules, idx: 0 }
    }
}

impl Iterator for ModulesIterator<'_> {
    type Item = char;

    fn next(&mut self) -> Option<Self::Item> {
        if self.modules[self.idx] == 0 {
            return None;
        }
        if self.idx < self.modules.len() {
            self.idx += 1;
            return Some(self.modules[self.idx - 1] as char);
        }
        None
    }
}

pub const MAGIC_VOICE: &[u8] = b"M17 ";
pub const MAGIC_VOICE_HEADER: &[u8] = b"M17H";
pub const MAGIC_VOICE_DATA: &[u8] = b"M17D";
pub const MAGIC_PACKET: &[u8] = b"M17P";
pub const MAGIC_ACKNOWLEDGE: &[u8] = b"ACKN";
pub const MAGIC_CONNECT: &[u8] = b"CONN";
pub const MAGIC_DISCONNECT: &[u8] = b"DISC";
pub const MAGIC_LISTEN: &[u8] = b"LSTN";
pub const MAGIC_NACK: &[u8] = b"NACK";
pub const MAGIC_PING: &[u8] = b"PING";
pub const MAGIC_PONG: &[u8] = b"PONG";

/// Messages sent from a station/client to a reflector
#[allow(clippy::large_enum_variant)]
pub enum ClientMessage {
    VoiceFull(VoiceFull),
    VoiceHeader(VoiceHeader),
    VoiceData(VoiceData),
    Packet(Packet),
    Pong(Pong),
    Connect(Connect),
    Listen(Listen),
    Disconnect(Disconnect),
}

/// Messages sent from a reflector to a station/client
#[allow(clippy::large_enum_variant)]
pub enum ServerMessage {
    VoiceFull(VoiceFull),
    VoiceHeader(VoiceHeader),
    VoiceData(VoiceData),
    Packet(Packet),
    Ping(Ping),
    DisconnectAcknowledge(DisconnectAcknowledge),
    ForceDisconnect(ForceDisconnect),
    ConnectAcknowledge(ConnectAcknowledge),
    ConnectNack(ConnectNack),
}

/// Messages sent and received between reflectors
#[allow(clippy::large_enum_variant)]
pub enum InterlinkMessage {
    VoiceInterlink(VoiceInterlink),
    VoiceHeaderInterlink(VoiceHeaderInterlink),
    VoiceDataInterlink(VoiceDataInterlink),
    PacketInterlink(PacketInterlink),
    Ping(Ping),
    ConnectInterlink(ConnectInterlink),
    ConnectInterlinkAcknowledge(ConnectInterlinkAcknowledge),
    ConnectNack(ConnectNack),
    DisconnectInterlink(DisconnectInterlink),
}

pub struct VoiceFull(pub [u8; 54]);
impl_stream_id!(VoiceFull, 4);
impl_link_setup!(VoiceFull, 6);
impl_frame_number!(VoiceFull, 34);
impl_payload!(VoiceFull, 36, 52);
impl_trailing_crc_verify!(VoiceFull);

pub struct VoiceHeader(pub [u8; 36]);
impl_stream_id!(VoiceHeader, 4);
impl_link_setup!(VoiceHeader, 6);
impl_trailing_crc_verify!(VoiceHeader);

pub struct VoiceData(pub [u8; 26]);
impl_stream_id!(VoiceData, 4);
impl_frame_number!(VoiceData, 6);
impl_payload!(VoiceData, 8, 24);
impl_trailing_crc_verify!(VoiceData);

pub struct Packet(pub [u8; 859]);
impl_link_setup_frame!(Packet, 4);

impl Packet {
    pub fn payload(&self) -> &[u8] {
        &self.0[34..]
    }

    pub fn verify_integrity(&self) -> bool {
        self.link_setup_frame().check_crc() == 0
            && self.payload().len() >= 4
            && crate::crc::m17_crc(self.payload()) == 0
    }
}

pub struct Pong(pub [u8; 10]);
impl_address!(Pong, 4);

pub struct Connect(pub [u8; 11]);
impl_address!(Connect, 4);
impl_module!(Connect, 10);

pub struct Listen(pub [u8; 11]);
impl_address!(Listen, 4);
impl_module!(Listen, 10);

pub struct Disconnect(pub [u8; 10]);
impl_address!(Disconnect, 4);

pub struct Ping(pub [u8; 10]);
impl_address!(Ping, 4);

pub struct DisconnectAcknowledge(pub [u8; 4]);

pub struct ForceDisconnect(pub [u8; 10]);
impl_address!(ForceDisconnect, 4);

pub struct ConnectAcknowledge(pub [u8; 4]);

pub struct ConnectNack(pub [u8; 4]);

pub struct VoiceInterlink(pub [u8; 55]);
impl_stream_id!(VoiceInterlink, 4);
impl_link_setup!(VoiceInterlink, 6);
impl_frame_number!(VoiceInterlink, 34);
impl_payload!(VoiceInterlink, 36, 52);
impl_internal_crc!(VoiceInterlink, 0, 54);
impl_is_relayed!(VoiceInterlink);

pub struct VoiceHeaderInterlink(pub [u8; 37]);
impl_stream_id!(VoiceHeaderInterlink, 4);
impl_link_setup!(VoiceHeaderInterlink, 6);
impl_internal_crc!(VoiceHeaderInterlink, 0, 36);
impl_is_relayed!(VoiceHeaderInterlink);

pub struct VoiceDataInterlink(pub [u8; 27]);
impl_stream_id!(VoiceDataInterlink, 4);
impl_frame_number!(VoiceDataInterlink, 6);
impl_payload!(VoiceDataInterlink, 8, 24);
impl_internal_crc!(VoiceDataInterlink, 0, 24);
impl_is_relayed!(VoiceDataInterlink);

pub struct PacketInterlink(pub [u8; 860]);
impl_link_setup_frame!(PacketInterlink, 4);
impl_is_relayed!(PacketInterlink);

impl PacketInterlink {
    pub fn payload(&self) -> &[u8] {
        &self.0[34..(self.0.len() - 1)]
    }

    pub fn verify_integrity(&self) -> bool {
        self.link_setup_frame().check_crc() == 0
            && self.payload().len() >= 4
            && crate::crc::m17_crc(self.payload()) == 0
    }
}

pub struct ConnectInterlink(pub [u8; 37]);
impl_address!(ConnectInterlink, 4);
impl_modules!(ConnectInterlink, 10, 37);

pub struct ConnectInterlinkAcknowledge(pub [u8; 37]);
impl_address!(ConnectInterlinkAcknowledge, 4);
impl_modules!(ConnectInterlinkAcknowledge, 10, 37);

pub struct DisconnectInterlink(pub [u8; 10]);
impl_address!(DisconnectInterlink, 4);
