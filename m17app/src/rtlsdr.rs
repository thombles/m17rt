use std::{
    io::Read,
    process::{Child, Command, Stdio},
    sync::{mpsc::SyncSender, Mutex},
};

use crate::{
    error::M17Error,
    soundmodem::{InputSource, SoundmodemErrorSender, SoundmodemEvent},
};

pub struct RtlSdr {
    frequency_mhz: f32,
    device_index: usize,
    rtlfm: Mutex<Option<Child>>,
}

impl RtlSdr {
    pub fn new(device_index: usize, frequency_mhz: f32) -> Result<Self, M17Error> {
        Ok(Self {
            device_index,
            frequency_mhz,
            rtlfm: Mutex::new(None),
        })
    }
}

impl InputSource for RtlSdr {
    fn start(&self, tx: SyncSender<SoundmodemEvent>, errors: SoundmodemErrorSender) {
        let mut cmd = match Command::new("rtl_fm")
            .args([
                "-E",
                "offset",
                "-f",
                &format!("{:.6}M", self.frequency_mhz),
                "-d",
                &self.device_index.to_string(),
                "-s",
                "48k",
            ])
            .stdout(Stdio::piped())
            .spawn()
        {
            Ok(c) => c,
            Err(e) => {
                errors.send_error(e);
                return;
            }
        };
        let mut stdout = cmd.stdout.take().unwrap();
        let mut buf = [0u8; 1024];
        let mut leftover: Option<u8> = None;
        std::thread::spawn(move || {
            while let Ok(n) = stdout.read(&mut buf) {
                let mut start_idx = 0;
                let mut samples = vec![];
                if let Some(left) = leftover {
                    if n > 0 {
                        samples.push(i16::from_le_bytes([left, buf[0]]));
                        start_idx = 1;
                        leftover = None;
                    }
                }
                for sample in buf[start_idx..n].chunks(2) {
                    if sample.len() == 2 {
                        samples.push(i16::from_le_bytes([sample[0], sample[1]]))
                    } else {
                        leftover = Some(sample[0]);
                    }
                }
                if tx
                    .send(SoundmodemEvent::BasebandInput(samples.into()))
                    .is_err()
                {
                    break;
                }
            }
        });
        *self.rtlfm.lock().unwrap() = Some(cmd);
    }

    fn close(&self) {
        if let Some(mut process) = self.rtlfm.lock().unwrap().take() {
            let _ = process.kill();
        }
    }
}
