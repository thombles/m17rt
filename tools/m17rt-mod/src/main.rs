use m17app::app::M17App;
use m17app::link_setup::M17Address;
use m17app::serial::{PttPin, SerialPtt};
use m17app::soundcard::Soundcard;
use m17app::soundmodem::{NullErrorHandler, Soundmodem};
use m17codec2::tx::WavePlayer;
use std::path::PathBuf;

pub fn mod_test() {
    let soundcard = Soundcard::new("plughw:CARD=Device,DEV=0").unwrap();
    soundcard.set_tx_inverted(true);
    let ptt = SerialPtt::new("/dev/ttyUSB0", PttPin::Rts).unwrap();
    let soundmodem = Soundmodem::new(
        soundcard.input(),
        soundcard.output(),
        ptt,
        NullErrorHandler::new(),
    );
    let app = M17App::new(soundmodem);
    app.start().unwrap();
    std::thread::sleep(std::time::Duration::from_secs(1));
    println!("Beginning playback...");
    WavePlayer::play(
        PathBuf::from("../../../Data/test_vk7xt_8k.wav"),
        app.tx(),
        &M17Address::from_callsign("VK7XT-1").unwrap(),
        &M17Address::new_broadcast(),
        0,
    );
    println!("Playback complete.");
    std::thread::sleep(std::time::Duration::from_secs(1));
    app.close().unwrap();
}

fn main() {
    env_logger::builder()
        .format_timestamp(Some(env_logger::TimestampPrecision::Millis))
        .init();
    mod_test();
}
