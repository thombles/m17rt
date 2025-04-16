// Based on https://github.com/n7tae/mrefd/blob/master/Packet-Description.md
// and the main M17 specification

use crate::protocol::LsfFrame;

macro_rules! define_message {
    ($t:tt, $sz:tt) => {
        pub struct $t([u8; $sz]);
        impl $t {
            pub fn from_bytes(b: &[u8]) -> Option<Self> {
                if b.len() != $sz {
                    return None;
                }
                let mut s = Self([0; $sz]);
                s.0[..].copy_from_slice(b);
                if !s.verify_integrity() {
                    return None;
                }
                Some(s)
            }
        }
    };
}

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

macro_rules! no_crc {
    ($t:ty) => {
        impl $t {
            pub fn verify_integrity(&self) -> bool {
                true
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
        if self.idx < self.modules.len() {
            if self.modules[self.idx] == 0 {
                return None;
            }
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

impl ClientMessage {
    pub fn parse(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 4 {
            return None;
        }
        match &bytes[0..4] {
            MAGIC_VOICE => Some(Self::VoiceFull(VoiceFull::from_bytes(&bytes)?)),
            MAGIC_VOICE_HEADER => Some(Self::VoiceHeader(VoiceHeader::from_bytes(&bytes)?)),
            MAGIC_VOICE_DATA => Some(Self::VoiceData(VoiceData::from_bytes(&bytes)?)),
            MAGIC_PACKET => Some(Self::Packet(Packet::from_bytes(&bytes)?)),
            MAGIC_PONG => Some(Self::Pong(Pong::from_bytes(&bytes)?)),
            MAGIC_CONNECT => Some(Self::Connect(Connect::from_bytes(&bytes)?)),
            MAGIC_LISTEN => Some(Self::Listen(Listen::from_bytes(&bytes)?)),
            MAGIC_DISCONNECT => Some(Self::Disconnect(Disconnect::from_bytes(&bytes)?)),
            _ => None,
        }
    }
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

impl ServerMessage {
    pub fn parse(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 4 {
            return None;
        }
        match &bytes[0..4] {
            MAGIC_VOICE => Some(Self::VoiceFull(VoiceFull::from_bytes(&bytes)?)),
            MAGIC_VOICE_HEADER => Some(Self::VoiceHeader(VoiceHeader::from_bytes(&bytes)?)),
            MAGIC_VOICE_DATA => Some(Self::VoiceData(VoiceData::from_bytes(&bytes)?)),
            MAGIC_PACKET => Some(Self::Packet(Packet::from_bytes(&bytes)?)),
            MAGIC_PING => Some(Self::Ping(Ping::from_bytes(&bytes)?)),
            MAGIC_DISCONNECT if bytes.len() == 4 => Some(Self::DisconnectAcknowledge(
                DisconnectAcknowledge::from_bytes(&bytes)?,
            )),
            MAGIC_DISCONNECT => Some(Self::ForceDisconnect(ForceDisconnect::from_bytes(&bytes)?)),
            MAGIC_ACKNOWLEDGE => Some(Self::ConnectAcknowledge(ConnectAcknowledge::from_bytes(
                &bytes,
            )?)),
            MAGIC_NACK => Some(Self::ConnectNack(ConnectNack::from_bytes(&bytes)?)),
            _ => None,
        }
    }
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

impl InterlinkMessage {
    pub fn parse(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 4 {
            return None;
        }
        match &bytes[0..4] {
            MAGIC_VOICE => Some(Self::VoiceInterlink(VoiceInterlink::from_bytes(&bytes)?)),
            MAGIC_VOICE_HEADER => Some(Self::VoiceHeaderInterlink(
                VoiceHeaderInterlink::from_bytes(&bytes)?,
            )),
            MAGIC_VOICE_DATA => Some(Self::VoiceDataInterlink(VoiceDataInterlink::from_bytes(
                &bytes,
            )?)),
            MAGIC_PACKET => Some(Self::PacketInterlink(PacketInterlink::from_bytes(&bytes)?)),
            MAGIC_PING => Some(Self::Ping(Ping::from_bytes(&bytes)?)),
            MAGIC_CONNECT => Some(Self::ConnectInterlink(ConnectInterlink::from_bytes(
                &bytes,
            )?)),
            MAGIC_ACKNOWLEDGE => Some(Self::ConnectInterlinkAcknowledge(
                ConnectInterlinkAcknowledge::from_bytes(&bytes)?,
            )),
            MAGIC_NACK => Some(Self::ConnectNack(ConnectNack::from_bytes(&bytes)?)),
            MAGIC_DISCONNECT => Some(Self::DisconnectInterlink(DisconnectInterlink::from_bytes(
                &bytes,
            )?)),
            _ => None,
        }
    }
}

define_message!(VoiceFull, 54);
impl_stream_id!(VoiceFull, 4);
impl_link_setup!(VoiceFull, 6);
impl_frame_number!(VoiceFull, 34);
impl_payload!(VoiceFull, 36, 52);
impl_trailing_crc_verify!(VoiceFull);

define_message!(VoiceHeader, 36);
impl_stream_id!(VoiceHeader, 4);
impl_link_setup!(VoiceHeader, 6);
impl_trailing_crc_verify!(VoiceHeader);

define_message!(VoiceData, 26);
impl_stream_id!(VoiceData, 4);
impl_frame_number!(VoiceData, 6);
impl_payload!(VoiceData, 8, 24);
impl_trailing_crc_verify!(VoiceData);

define_message!(Packet, 859);
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

define_message!(Pong, 10);
impl_address!(Pong, 4);
no_crc!(Pong);

define_message!(Connect, 11);
impl_address!(Connect, 4);
impl_module!(Connect, 10);
no_crc!(Connect);

define_message!(Listen, 11);
impl_address!(Listen, 4);
impl_module!(Listen, 10);
no_crc!(Listen);

define_message!(Disconnect, 10);
impl_address!(Disconnect, 4);
no_crc!(Disconnect);

define_message!(Ping, 10);
impl_address!(Ping, 4);
no_crc!(Ping);

define_message!(DisconnectAcknowledge, 4);
no_crc!(DisconnectAcknowledge);

define_message!(ForceDisconnect, 10);
impl_address!(ForceDisconnect, 4);
no_crc!(ForceDisconnect);

define_message!(ConnectAcknowledge, 4);
no_crc!(ConnectAcknowledge);

define_message!(ConnectNack, 4);
no_crc!(ConnectNack);

define_message!(VoiceInterlink, 55);
impl_stream_id!(VoiceInterlink, 4);
impl_link_setup!(VoiceInterlink, 6);
impl_frame_number!(VoiceInterlink, 34);
impl_payload!(VoiceInterlink, 36, 52);
impl_internal_crc!(VoiceInterlink, 0, 54);
impl_is_relayed!(VoiceInterlink);

define_message!(VoiceHeaderInterlink, 37);
impl_stream_id!(VoiceHeaderInterlink, 4);
impl_link_setup!(VoiceHeaderInterlink, 6);
impl_internal_crc!(VoiceHeaderInterlink, 0, 36);
impl_is_relayed!(VoiceHeaderInterlink);

define_message!(VoiceDataInterlink, 27);
impl_stream_id!(VoiceDataInterlink, 4);
impl_frame_number!(VoiceDataInterlink, 6);
impl_payload!(VoiceDataInterlink, 8, 24);
impl_internal_crc!(VoiceDataInterlink, 0, 24);
impl_is_relayed!(VoiceDataInterlink);

define_message!(PacketInterlink, 860);
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

define_message!(ConnectInterlink, 37);
impl_address!(ConnectInterlink, 4);
impl_modules!(ConnectInterlink, 10, 37);
no_crc!(ConnectInterlink);

define_message!(ConnectInterlinkAcknowledge, 37);
impl_address!(ConnectInterlinkAcknowledge, 4);
impl_modules!(ConnectInterlinkAcknowledge, 10, 37);
no_crc!(ConnectInterlinkAcknowledge);

define_message!(DisconnectInterlink, 10);
impl_address!(DisconnectInterlink, 4);
no_crc!(DisconnectInterlink);
