#![allow(clippy::needless_range_loop)]
#![cfg_attr(not(test), no_std)]

pub mod address;
pub mod kiss;
pub mod modem;
pub mod protocol;
pub mod tnc;
pub mod traits;

mod bits;
mod crc;
mod decode;
mod fec;
mod interleave;
mod random;
mod shaping;
