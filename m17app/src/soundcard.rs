use std::{
    sync::{
        mpsc::{sync_channel, Receiver, SyncSender},
        Arc, RwLock,
    },
    time::{Duration, Instant},
};

use cpal::{
    traits::{DeviceTrait, HostTrait, StreamTrait},
    BuildStreamError, DevicesError, PlayStreamError, SampleFormat, SampleRate, Stream, StreamError,
    SupportedStreamConfigsError,
};
use thiserror::Error;

use crate::soundmodem::{
    InputSource, OutputBuffer, OutputSink, SoundmodemErrorSender, SoundmodemEvent,
};

pub struct Soundcard {
    event_tx: SyncSender<SoundcardEvent>,
}

impl Soundcard {
    pub fn new<S: Into<String>>(card_name: S) -> Result<Self, SoundcardError> {
        let (card_tx, card_rx) = sync_channel(128);
        let (setup_tx, setup_rx) = sync_channel(1);
        spawn_soundcard_worker(card_rx, setup_tx, card_name.into());
        match setup_rx.recv() {
            Ok(Ok(())) => Ok(Self { event_tx: card_tx }),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(SoundcardError::SoundcardInit),
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
        errors: SoundmodemErrorSender,
    },
    CloseInput,
    StartOutput {
        event_tx: SyncSender<SoundmodemEvent>,
        buffer: Arc<RwLock<OutputBuffer>>,
        errors: SoundmodemErrorSender,
    },
    CloseOutput,
}

pub struct SoundcardInputSource {
    event_tx: SyncSender<SoundcardEvent>,
}

impl InputSource for SoundcardInputSource {
    fn start(&self, samples: SyncSender<SoundmodemEvent>, errors: SoundmodemErrorSender) {
        let _ = self
            .event_tx
            .send(SoundcardEvent::StartInput { samples, errors });
    }

    fn close(&self) {
        let _ = self.event_tx.send(SoundcardEvent::CloseInput);
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
        errors: SoundmodemErrorSender,
    ) {
        let _ = self.event_tx.send(SoundcardEvent::StartOutput {
            event_tx,
            buffer,
            errors,
        });
    }

    fn close(&self) {
        let _ = self.event_tx.send(SoundcardEvent::CloseOutput);
    }
}

fn spawn_soundcard_worker(
    event_rx: Receiver<SoundcardEvent>,
    setup_tx: SyncSender<Result<(), SoundcardError>>,
    card_name: String,
) {
    std::thread::spawn(move || {
        let host = cpal::default_host();
        let Some(device) = host
            .devices()
            .unwrap()
            .find(|d| d.name().unwrap() == card_name)
        else {
            let _ = setup_tx.send(Err(SoundcardError::CardNotFound(card_name)));
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
                SoundcardEvent::StartInput { samples, errors } => {
                    let mut input_configs = match device.supported_input_configs() {
                        Ok(c) => c,
                        Err(e) => {
                            errors.send_error(SoundcardError::SupportedConfigs(e));
                            continue;
                        }
                    };
                    let input_config = match input_configs
                        .find(|c| c.channels() == 1 && c.sample_format() == SampleFormat::I16)
                    {
                        Some(c) => c,
                        None => {
                            errors.send_error(SoundcardError::NoValidConfigAvailable);
                            continue;
                        }
                    };
                    let input_config = input_config.with_sample_rate(SampleRate(48000));
                    let errors_1 = errors.clone();
                    let stream = match device.build_input_stream(
                        &input_config.into(),
                        move |data: &[i16], _info: &cpal::InputCallbackInfo| {
                            let out: Vec<i16> = data
                                .iter()
                                .map(|s| if rx_inverted { s.saturating_neg() } else { *s })
                                .collect();
                            let _ = samples.try_send(SoundmodemEvent::BasebandInput(out.into()));
                        },
                        move |e| {
                            errors_1.send_error(SoundcardError::Stream(e));
                        },
                        None,
                    ) {
                        Ok(s) => s,
                        Err(e) => {
                            errors.send_error(SoundcardError::StreamBuild(e));
                            continue;
                        }
                    };
                    if let Err(e) = stream.play() {
                        errors.send_error(SoundcardError::StreamPlay(e));
                        continue;
                    }
                    input_stream = Some(stream);
                }
                SoundcardEvent::CloseInput => {
                    let _ = input_stream.take();
                }
                SoundcardEvent::StartOutput {
                    event_tx,
                    buffer,
                    errors,
                } => {
                    let mut output_configs = match device.supported_output_configs() {
                        Ok(c) => c,
                        Err(e) => {
                            errors.send_error(SoundcardError::SupportedConfigs(e));
                            continue;
                        }
                    };
                    let output_config = match output_configs
                        .find(|c| c.channels() == 1 && c.sample_format() == SampleFormat::I16)
                    {
                        Some(c) => c,
                        None => {
                            errors.send_error(SoundcardError::NoValidConfigAvailable);
                            continue;
                        }
                    };
                    let output_config = output_config.with_sample_rate(SampleRate(48000));
                    let errors_1 = errors.clone();
                    let stream = match device.build_output_stream(
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
                                    let _ = event_tx.send(SoundmodemEvent::OutputUnderrun);
                                    break;
                                }
                            }
                            let _ = event_tx.send(SoundmodemEvent::DidReadFromOutputBuffer {
                                len: taken,
                                timestamp: Instant::now(),
                            });
                        },
                        move |e| {
                            errors_1.send_error(SoundcardError::Stream(e));
                        },
                        None,
                    ) {
                        Ok(s) => s,
                        Err(e) => {
                            errors.send_error(SoundcardError::StreamBuild(e));
                            continue;
                        }
                    };
                    if let Err(e) = stream.play() {
                        errors.send_error(SoundcardError::StreamPlay(e));
                        continue;
                    }
                    output_stream = Some(stream);
                }
                SoundcardEvent::CloseOutput => {
                    let _ = output_stream.take();
                }
            }
        }
    });
}

#[derive(Debug, Error)]
pub enum SoundcardError {
    #[error("sound card init aborted unexpectedly")]
    SoundcardInit,

    #[error("unable to enumerate devices: {0}")]
    Host(DevicesError),

    #[error("unable to locate sound card '{0}' - is it in use?")]
    CardNotFound(String),

    #[error("error occurred in soundcard i/o: {0}")]
    Stream(#[source] StreamError),

    #[error("unable to retrieve supported configs for soundcard: {0}")]
    SupportedConfigs(#[source] SupportedStreamConfigsError),

    #[error("could not find a suitable soundcard config")]
    NoValidConfigAvailable,

    #[error("unable to build soundcard stream: {0}")]
    StreamBuild(#[source] BuildStreamError),

    #[error("unable to play stream")]
    StreamPlay(#[source] PlayStreamError),
}
