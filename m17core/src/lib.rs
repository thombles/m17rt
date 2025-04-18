#![doc = include_str!("../README.md")]
#![allow(clippy::needless_range_loop)]
#![cfg_attr(not(test), no_std)]

pub mod address;
pub mod crc;
pub mod kiss;
pub mod modem;
pub mod protocol;
pub mod reflector;
pub mod tnc;

mod bits;
mod decode;
mod encode;
mod fec;
mod interleave;
mod random;
mod shaping;
