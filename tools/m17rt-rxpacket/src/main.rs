use m17app::adapter::PacketAdapter;
use m17app::app::M17App;
use m17app::link_setup::LinkSetup;
use m17app::soundmodem::{InputRrcFile, NullOutputSink, NullPtt, Soundmodem};
use m17core::protocol::PacketType;
use std::path::PathBuf;
use std::sync::Arc;

fn main() {
    let path = PathBuf::from("../../../Data/mypacket.rrc");
    let soundmodem = Soundmodem::new(
        InputRrcFile::new(path),
        NullOutputSink::new(),
        NullPtt::new(),
    );
    let app = M17App::new(soundmodem);
    app.add_packet_adapter(PacketPrinter);
    app.start();

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
