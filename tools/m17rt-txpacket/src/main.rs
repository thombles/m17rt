use m17app::app::M17App;
use m17app::link_setup::{LinkSetup, M17Address};
use m17app::soundmodem::{
    InputRrcFile, InputSoundcard, NullInputSource, NullOutputSink, NullPtt, OutputRrcFile,
    OutputSoundcard, Soundmodem,
};
use m17core::protocol::PacketType;
use std::path::PathBuf;

pub fn mod_test() {
    let out_path = PathBuf::from("../../../Data/mypacket.rrc");
    let output = OutputRrcFile::new(out_path);
    //let output = OutputSoundcard::new();
    let soundmodem = Soundmodem::new(NullInputSource::new(), output, NullPtt::new());
    let app = M17App::new(soundmodem);
    app.start();
    std::thread::sleep(std::time::Duration::from_secs(1));
    println!("Transmitting packet...");

    let source = M17Address::from_callsign("VK7XT").unwrap();
    let destination = M17Address::new_broadcast();
    let link_setup = LinkSetup::new_packet(&source, &destination);
    let payload = b"Hello, world!";
    app.tx()
        .transmit_packet(&link_setup, PacketType::Raw, payload);

    std::thread::sleep(std::time::Duration::from_secs(5));
}

fn main() {
    env_logger::init();
    mod_test();
}
