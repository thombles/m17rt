use serialport::SerialPort;

use crate::{error::SoundmodemError, soundmodem::Ptt};

/// The pin on the serial port which is driving PTT
pub enum PttPin {
    // Ready To Send (RTS)
    Rts,
    // Data Terminal ready (DTR)
    Dtr,
}

pub struct SerialPtt {
    port: Box<dyn SerialPort>,
    pin: PttPin,
}

impl SerialPtt {
    pub fn available_ports() -> impl Iterator<Item = String> {
        serialport::available_ports()
            .unwrap_or_else(|_| vec![])
            .into_iter()
            .map(|i| i.port_name)
    }

    pub fn new(port_name: &str, pin: PttPin) -> Result<Self, SoundmodemError> {
        let port = serialport::new(port_name, 9600).open()?;
        let mut s = Self { port, pin };
        s.ptt_off()?;
        Ok(s)
    }
}

impl Ptt for SerialPtt {
    fn ptt_on(&mut self) -> Result<(), SoundmodemError> {
        Ok(match self.pin {
            PttPin::Rts => self.port.write_request_to_send(true),
            PttPin::Dtr => self.port.write_data_terminal_ready(true),
        }?)
    }

    fn ptt_off(&mut self) -> Result<(), SoundmodemError> {
        Ok(match self.pin {
            PttPin::Rts => self.port.write_request_to_send(false),
            PttPin::Dtr => self.port.write_data_terminal_ready(false),
        }?)
    }
}
