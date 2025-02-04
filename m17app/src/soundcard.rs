use std::{
    sync::{
        mpsc::{sync_channel, Receiver, SyncSender},
        Arc, RwLock,
    },
    time::{Duration, Instant},
};

use cpal::{
    traits::{DeviceTrait, HostTrait, StreamTrait},
    SampleFormat, SampleRate, Stream,
};

use crate::{
    error::{M17Error, SoundmodemError},
    soundmodem::{InputSource, OutputBuffer, OutputSink, SoundmodemEvent},
};

pub struct Soundcard {
    event_tx: SyncSender<SoundcardEvent>,
}

impl Soundcard {
    pub fn new<S: Into<String>>(card_name: S) -> Result<Self, M17Error> {
        let (card_tx, card_rx) = sync_channel(128);
        let (setup_tx, setup_rx) = sync_channel(1);
        spawn_soundcard_worker(card_rx, setup_tx, card_name.into());
        match setup_rx.recv() {
            Ok(Ok(())) => Ok(Self { event_tx: card_tx }),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(M17Error::SoundcardInit),
        }
    }

    pub fn input(&self) -> SoundcardInputSource {
        SoundcardInputSource {
            event_tx: self.event_tx.clone(),
        }
    }

    pub fn output(&self) -> SoundcardOutputSink {
        SoundcardOutputSink {
            event_tx: self.event_tx.clone(),
        }
    }

    pub fn set_rx_inverted(&self, inverted: bool) {
        let _ = self.event_tx.send(SoundcardEvent::SetRxInverted(inverted));
    }

    pub fn set_tx_inverted(&self, inverted: bool) {
        let _ = self.event_tx.send(SoundcardEvent::SetTxInverted(inverted));
    }

    pub fn supported_output_cards() -> Vec<String> {
        let mut out = vec![];
        let host = cpal::default_host();
        let Ok(output_devices) = host.output_devices() else {
            return out;
        };
        for d in output_devices {
            let Ok(mut configs) = d.supported_output_configs() else {
                continue;
            };
            if configs
                .any(|config| config.channels() == 1 && config.sample_format() == SampleFormat::I16)
            {
                let Ok(name) = d.name() else {
                    continue;
                };
                out.push(name);
            }
        }
        out.sort();
        out
    }

    pub fn supported_input_cards() -> Vec<String> {
        let mut out = vec![];
        let host = cpal::default_host();
        let Ok(input_devices) = host.input_devices() else {
            return out;
        };
        for d in input_devices {
            let Ok(mut configs) = d.supported_input_configs() else {
                continue;
            };
            if configs
                .any(|config| config.channels() == 1 && config.sample_format() == SampleFormat::I16)
            {
                let Ok(name) = d.name() else {
                    continue;
                };
                out.push(name);
            }
        }
        out.sort();
        out
    }
}

enum SoundcardEvent {
    SetRxInverted(bool),
    SetTxInverted(bool),
    StartInput {
        samples: SyncSender<SoundmodemEvent>,
    },
    CloseInput,
    StartOutput {
        event_tx: SyncSender<SoundmodemEvent>,
        buffer: Arc<RwLock<OutputBuffer>>,
    },
    CloseOutput,
}

pub struct SoundcardInputSource {
    event_tx: SyncSender<SoundcardEvent>,
}

impl InputSource for SoundcardInputSource {
    fn start(&self, samples: SyncSender<SoundmodemEvent>) -> Result<(), SoundmodemError> {
        Ok(self.event_tx.send(SoundcardEvent::StartInput { samples })?)
    }

    fn close(&self) -> Result<(), SoundmodemError> {
        Ok(self.event_tx.send(SoundcardEvent::CloseInput)?)
    }
}

pub struct SoundcardOutputSink {
    event_tx: SyncSender<SoundcardEvent>,
}

impl OutputSink for SoundcardOutputSink {
    fn start(
        &self,
        event_tx: SyncSender<SoundmodemEvent>,
        buffer: Arc<RwLock<OutputBuffer>>,
    ) -> Result<(), SoundmodemError> {
        Ok(self
            .event_tx
            .send(SoundcardEvent::StartOutput { event_tx, buffer })?)
    }

    fn close(&self) -> Result<(), SoundmodemError> {
        Ok(self.event_tx.send(SoundcardEvent::CloseOutput)?)
    }
}

fn spawn_soundcard_worker(
    event_rx: Receiver<SoundcardEvent>,
    setup_tx: SyncSender<Result<(), M17Error>>,
    card_name: String,
) {
    std::thread::spawn(move || {
        let host = cpal::default_host();
        let Some(device) = host
            .devices()
            .unwrap()
            .find(|d| d.name().unwrap() == card_name)
        else {
            let _ = setup_tx.send(Err(M17Error::SoundcardNotFound(card_name)));
            return;
        };

        let _ = setup_tx.send(Ok(()));
        let mut rx_inverted = false;
        let mut tx_inverted = false;
        let mut input_stream: Option<Stream> = None;
        let mut output_stream: Option<Stream> = None;

        while let Ok(ev) = event_rx.recv() {
            match ev {
                SoundcardEvent::SetRxInverted(inv) => rx_inverted = inv,
                SoundcardEvent::SetTxInverted(inv) => tx_inverted = inv,
                SoundcardEvent::StartInput { samples } => {
                    let mut input_configs = device.supported_input_configs().unwrap();
                    let input_config = input_configs
                        .find(|c| c.channels() == 1 && c.sample_format() == SampleFormat::I16)
                        .unwrap()
                        .with_sample_rate(SampleRate(48000));
                    let stream = device
                        .build_input_stream(
                            &input_config.into(),
                            move |data: &[i16], _info: &cpal::InputCallbackInfo| {
                                let out: Vec<i16> = data
                                    .iter()
                                    .map(|s| if rx_inverted { s.saturating_neg() } else { *s })
                                    .collect();
                                let _ =
                                    samples.try_send(SoundmodemEvent::BasebandInput(out.into()));
                            },
                            |e| {
                                // TODO: abort?
                                log::debug!("error occurred in soundcard input: {e:?}");
                            },
                            None,
                        )
                        .unwrap();
                    stream.play().unwrap();
                    input_stream = Some(stream);
                }
                SoundcardEvent::CloseInput => {
                    let _ = input_stream.take();
                }
                SoundcardEvent::StartOutput { event_tx, buffer } => {
                    let mut output_configs = device.supported_output_configs().unwrap();
                    // TODO: more error handling
                    let output_config = output_configs
                        .find(|c| c.channels() == 1 && c.sample_format() == SampleFormat::I16)
                        .unwrap()
                        .with_sample_rate(SampleRate(48000));
                    let stream = device
                        .build_output_stream(
                            &output_config.into(),
                            move |data: &mut [i16], info: &cpal::OutputCallbackInfo| {
                                let mut taken = 0;
                                let ts = info.timestamp();
                                let latency = ts
                                    .playback
                                    .duration_since(&ts.callback)
                                    .unwrap_or(Duration::ZERO);
                                let mut buffer = buffer.write().unwrap();
                                buffer.latency = latency;
                                for out in data.iter_mut() {
                                    if let Some(s) = buffer.samples.pop_front() {
                                        *out = if tx_inverted { s.saturating_neg() } else { s };
                                        taken += 1;
                                    } else if buffer.idling {
                                        *out = 0;
                                    } else {
                                        log::debug!("output soundcard had underrun");
                                        let _ = event_tx.send(SoundmodemEvent::OutputUnderrun);
                                        break;
                                    }
                                }
                                //debug!("latency is {} ms, taken {taken}", latency.as_millis());
                                let _ = event_tx.send(SoundmodemEvent::DidReadFromOutputBuffer {
                                    len: taken,
                                    timestamp: Instant::now(),
                                });
                            },
                            |e| {
                                // TODO: abort?
                                log::debug!("error occurred in soundcard output: {e:?}");
                            },
                            None,
                        )
                        .unwrap();
                    stream.play().unwrap();
                    output_stream = Some(stream);
                }
                SoundcardEvent::CloseOutput => {
                    let _ = output_stream.take();
                }
            }
        }
    });
}
