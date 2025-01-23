use m17app::app::M17App;
use m17app::link_setup::{LinkSetup, M17Address};
use m17app::serial::{PttPin, SerialPtt};
use m17app::soundcard::Soundcard;
use m17app::soundmodem::Soundmodem;
use m17core::protocol::PacketType;

fn main() {
    let soundcard = Soundcard::new("plughw:CARD=Device,DEV=0").unwrap();
    let ptt = SerialPtt::new("/dev/ttyUSB0", PttPin::Rts);
    let soundmodem = Soundmodem::new(soundcard.input(), soundcard.output(), ptt);
    let app = M17App::new(soundmodem);

    app.start();

    println!("Transmitting packet...");
    let source = M17Address::from_callsign("VK7XT-1").unwrap();
    let destination = M17Address::new_broadcast();
    let link_setup = LinkSetup::new_packet(&source, &destination);
    let payload = b"Hello, world!";
    app.tx()
        .transmit_packet(&link_setup, PacketType::Sms, payload);

    std::thread::sleep(std::time::Duration::from_secs(1));
    app.close();
}
