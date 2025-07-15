use std::{error::Error, fs::File, io::Read, path::PathBuf};

use clap::Parser;
use m17core::{
    modem::{Demodulator, SoftDemodulator},
    protocol::Frame,
};

#[derive(Parser)]
struct Args {
    #[arg(short = 'i', help = "Input RRC file")]
    input: PathBuf,
}

fn main() -> Result<(), Box<dyn Error>> {
    env_logger::init();
    let args = Args::parse();

    let mut file = File::open(&args.input)?;
    let mut baseband = vec![];
    file.read_to_end(&mut baseband)?;

    let mut total = 0;
    let mut demod = SoftDemodulator::new();
    for (idx, sample) in baseband
        .chunks(2)
        .map(|pair| i16::from_le_bytes([pair[0], pair[1]]))
        .enumerate()
    {
        if let Some((frame, errors)) = demod.demod(sample) {
            total += 1;
            let frame_desc = match frame {
                Frame::Lsf(_) => "lsf",
                Frame::Stream(_) => "stream",
                Frame::Packet(_) => "packet",
            };
            println!("sample {}: {} with {} errors", idx, frame_desc, errors);
        }
    }

    println!("\ntotal successful decodes: {}", total);

    Ok(())
}
