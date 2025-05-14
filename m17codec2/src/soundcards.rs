//! Utilities for selecting suitable sound cards.

use cpal::{
    traits::{DeviceTrait, HostTrait},
    SampleFormat,
};

/// List sound cards supported for audio output.
///
/// M17RT will handle any card with 1 or 2 channels and 16-bit output.
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
        if configs.any(|config| {
            (config.channels() == 1 || config.channels() == 2)
                && config.sample_format() == SampleFormat::I16
        }) {
            let Ok(name) = d.name() else {
                continue;
            };
            out.push(name);
        }
    }
    out.sort();
    out
}

/// List sound cards supported for audio input.
///
///
/// M17RT will handle any card with 1 or 2 channels and 16-bit output.
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
        if configs.any(|config| {
            (config.channels() == 1 || config.channels() == 2)
                && config.sample_format() == SampleFormat::I16
        }) {
            let Ok(name) = d.name() else {
                continue;
            };
            out.push(name);
        }
    }
    out.sort();
    out
}
