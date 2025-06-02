#![doc = include_str!("../README.md")]

pub mod adapter;
pub mod app;
pub mod error;
pub mod link_setup;
pub mod reflector;
pub mod rtlsdr;
pub mod serial;
pub mod soundcard;
pub mod soundmodem;
pub mod tnc;
pub mod util;

#[cfg(test)]
mod test_util;

// Protocol definitions needed to implement stream and packet adapters or create fully custom LSFs
pub use m17core::protocol::{LsfFrame, PacketType, StreamFrame};
