pub(crate) use codec2::{Codec2, Codec2Mode};
use std::fs::File;
use std::io::Write;
use std::path::Path;

pub fn decode_codec2<P: AsRef<Path>>(data: &[u8], out_path: P) {
    let codec2 = Codec2::new(Codec2Mode::MODE_3200);
    let var_name = codec2;
    let mut codec = var_name;
    let mut all_samples: Vec<i16> = vec![];
    for i in 0..(data.len() / 8) {
        let mut samples = vec![0; codec.samples_per_frame()];
        codec.decode(&mut samples, &data[i * 8..((i + 1) * 8)]);
        all_samples.append(&mut samples);
    }

    // dude this works
    let mut speech_out = File::create(out_path).unwrap();
    for b in all_samples {
        speech_out.write_all(&b.to_le_bytes()).unwrap();
    }
}
