use cpal::traits::DeviceTrait;
use cpal::traits::HostTrait;
use cpal::traits::StreamTrait;
use cpal::{SampleFormat, SampleRate};
use log::debug;
use m17core::{
    modem::{Demodulator, SoftDemodulator},
    protocol::{Frame, LichCollection},
};
use std::{fs::File, io::Read};

pub fn run_my_decode() {
    let file = File::open("../../Data/test_vk7xt.rrc").unwrap();
    let mut input = file;
    let mut baseband = vec![];
    input.read_to_end(&mut baseband).unwrap();

    let mut lich = LichCollection::new();
    let mut codec2_data = vec![];
    let mut modem = SoftDemodulator::new();

    for pair in baseband.chunks(2) {
        let sample: i16 = i16::from_le_bytes([pair[0], pair[1]]);
        if let Some(frame) = modem.demod(sample) {
            debug!("Modem demodulated frame: {:?}", frame);
            if let Frame::Stream(s) = frame {
                for b in s.stream_data {
                    codec2_data.push(b);

                    let valid_before = lich.valid_segments();
                    lich.set_segment(s.lich_idx, s.lich_part);
                    let valid_after = lich.valid_segments();
                    if valid_before != valid_after {
                        debug!("Valid lich segments: {}", lich.valid_segments());
                    }
                    if valid_before == 5 && valid_after == 6 {
                        if let Some(l) = lich.try_assemble() {
                            debug!("Assembled complete lich: {l:?}");
                        }
                    }
                }
                if s.end_of_stream {
                    debug!("len of codec2 data: {}", codec2_data.len());
                    assert_eq!(codec2_data.len(), 1504);

                    let samples =
                        m17codec2::decode_codec2(&codec2_data, "../../Data/speech_out.raw");
                    let host = cpal::default_host();
                    let def = host.default_output_device().unwrap();
                    let mut configs = def.supported_output_configs().unwrap();
                    let config = configs
                        .find(|c| c.channels() == 1 && c.sample_format() == SampleFormat::I16)
                        .unwrap()
                        .with_sample_rate(SampleRate(8000));
                    let mut counter = 0;
                    let mut index = 0;
                    let stream = def
                        .build_output_stream(
                            &config.into(),
                            move |data: &mut [i16], info: &cpal::OutputCallbackInfo| {
                                debug!(
                                    "callback {:?} playback {:?}",
                                    info.timestamp().callback,
                                    info.timestamp().playback
                                );
                                println!(
                                    "iteration {counter} asked for {} samples at time {}",
                                    data.len(),
                                    std::time::SystemTime::now()
                                        .duration_since(std::time::UNIX_EPOCH)
                                        .unwrap()
                                        .as_millis()
                                );
                                counter += 1;
                                let qty = data.len().min(samples.len() - index);
                                println!("providing {qty} samples");
                                data[0..qty].copy_from_slice(&samples[index..(index + qty)]);
                                index += qty;
                            },
                            move |_e| {
                                println!("error occurred");
                            },
                            None,
                        )
                        .unwrap();
                    stream.play().unwrap();

                    std::thread::sleep(std::time::Duration::from_secs(10));
                }
            }
        }
    }
}

pub fn cpal_test() {
    let host = cpal::default_host();
    for d in host.devices().unwrap() {
        println!("Found card: {:?}", d.name().unwrap());
    }
    let def = host.default_output_device().unwrap();
    println!("the default output device is: {}", def.name().unwrap());

    for c in def.supported_output_configs().unwrap() {
        println!("config supported: {:?}", c);
    }

    println!("all supported output configs shown");
}

fn main() {
    env_logger::init();
    run_my_decode();
    //cpal_test();
}
