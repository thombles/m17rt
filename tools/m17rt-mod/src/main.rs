use m17app::app::M17App;
use m17app::link_setup::M17Address;
use m17app::soundmodem::{
    InputRrcFile, InputSoundcard, NullInputSource, NullOutputSink, NullPtt, OutputRrcFile,
    OutputSoundcard, Soundmodem,
};
use m17codec2::{Codec2Adapter, WavePlayer};
use std::path::PathBuf;

pub fn mod_test() {
    let in_path = PathBuf::from("../../../Data/test_vk7xt_8k.wav");
    let out_path = PathBuf::from("../../../Data/mymod.rrc");
    let output = OutputRrcFile::new(out_path);
    //let output = OutputSoundcard::new();
    let soundmodem = Soundmodem::new(NullInputSource::new(), output, NullPtt::new());
    let app = M17App::new(soundmodem);
    app.start();
    std::thread::sleep(std::time::Duration::from_secs(1));
    println!("Beginning playback...");
    WavePlayer::play(
        in_path,
        app.tx(),
        &M17Address::from_callsign("VK7XT").unwrap(),
        &M17Address::new_broadcast(),
        0,
    );
    println!("Playback complete, terminating in 5 secs");
    std::thread::sleep(std::time::Duration::from_secs(5));
}

fn main() {
    env_logger::init();
    mod_test();
}
