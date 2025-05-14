use std::fmt::Display;

use m17core::{
    address::{Address, Callsign, ALPHABET},
    protocol::LsfFrame,
};

use crate::error::M17Error;

pub struct LinkSetup {
    pub(crate) raw: LsfFrame,
}

impl LinkSetup {
    /// Provide a completed LsfFrame.
    pub fn new_raw(frame: LsfFrame) -> Self {
        Self { raw: frame }
    }

    pub fn source(&self) -> M17Address {
        M17Address(self.raw.source())
    }

    pub fn destination(&self) -> M17Address {
        M17Address(self.raw.destination())
    }

    /// Set up an unencrypted voice stream with channel access number 0 and the given source and destination.
    pub fn new_voice(source: &M17Address, destination: &M17Address) -> Self {
        Self {
            raw: LsfFrame::new_voice(source.address(), destination.address()),
        }
    }

    /// Set up an unencrypted packet data transmission with channel access number 0 and the given source and destination.
    pub fn new_packet(source: &M17Address, destination: &M17Address) -> Self {
        Self {
            raw: LsfFrame::new_packet(source.address(), destination.address()),
        }
    }

    /// Configure the channel access number for this transmission, which may be from 0 to 15 inclusive.
    pub fn set_channel_access_number(&mut self, channel_access_number: u8) {
        self.raw.set_channel_access_number(channel_access_number);
    }

    pub fn lich_part(&self, counter: u8) -> [u8; 5] {
        let idx = counter as usize;
        self.raw.0[idx * 5..(idx + 1) * 5].try_into().unwrap()
    }
}

/// Station address. High level version of `Address` from core.

#[derive(Debug, Clone)]
pub struct M17Address(Address);

impl M17Address {
    pub fn new_broadcast() -> Self {
        Self(Address::Broadcast)
    }

    pub fn from_callsign(callsign: &str) -> Result<Self, M17Error> {
        let trimmed = callsign.trim().to_uppercase();
        let len = trimmed.len();
        if len > 9 {
            return Err(M17Error::CallsignTooLong(len));
        }
        let mut address = [b' '; 9];
        for (i, c) in trimmed.chars().enumerate() {
            if !c.is_ascii() {
                return Err(M17Error::InvalidCallsignCharacters(c));
            }
            if !ALPHABET.contains(&(c as u8)) {
                return Err(M17Error::InvalidCallsignCharacters(c));
            }
            address[i] = c as u8;
        }
        Ok(Self(Address::Callsign(Callsign(address))))
    }

    pub fn address(&self) -> &Address {
        &self.0
    }
}

impl Display for M17Address {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.0 {
            Address::Invalid => unreachable!(),
            Address::Callsign(ref callsign) => {
                write!(
                    f,
                    "{}",
                    callsign
                        .0
                        .iter()
                        .map(|c| *c as char)
                        .collect::<String>()
                        .trim()
                )
            }
            Address::Reserved(_) => unreachable!(),
            Address::Broadcast => {
                write!(f, "<BROADCAST>")
            }
        }
    }
}
