use m17app::adapter::PacketAdapter;
use m17app::app::M17App;
use m17app::link_setup::LinkSetup;
use m17app::soundcard::Soundcard;
use m17app::soundmodem::{NullErrorHandler, NullOutputSink, NullPtt, Soundmodem};
use m17app::PacketType;
use std::sync::Arc;

fn main() {
    let soundcard = Soundcard::new("plughw:CARD=Device,DEV=0").unwrap();
    let soundmodem = Soundmodem::new(
        soundcard.input(),
        NullOutputSink::new(),
        NullPtt::new(),
        NullErrorHandler::new(),
    );
    let app = M17App::new(soundmodem);
    app.add_packet_adapter(PacketPrinter).unwrap();
    app.start().unwrap();

    loop {
        std::thread::park();
    }
}

struct PacketPrinter;
impl PacketAdapter for PacketPrinter {
    fn packet_received(&self, link_setup: LinkSetup, packet_type: PacketType, content: Arc<[u8]>) {
        println!(
            "from {} to {} type {:?} len {}",
            link_setup.source(),
            link_setup.destination(),
            packet_type,
            content.len()
        );
        println!("{}", String::from_utf8_lossy(&content));
    }
}
