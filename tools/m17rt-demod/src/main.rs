use m17app::app::M17App;
use m17app::soundcard::Soundcard;
use m17app::soundmodem::{NullOutputSink, NullPtt, Soundmodem};
use m17codec2::Codec2Adapter;

pub fn demod_test() {
    let soundcard = Soundcard::new("plughw:CARD=Device,DEV=0").unwrap();
    let soundmodem = Soundmodem::new(soundcard.input(), NullOutputSink::new(), NullPtt::new());
    let app = M17App::new(soundmodem);
    app.add_stream_adapter(Codec2Adapter::new()).unwrap();
    app.start().unwrap();

    loop {
        std::thread::park();
    }
}

fn main() {
    env_logger::init();
    demod_test();
}
