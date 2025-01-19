use m17app::app::M17App;
use m17app::soundmodem::{InputRrcFile, InputSoundcard, NullOutputSink, NullPtt, Soundmodem};
use m17codec2::Codec2Adapter;
use std::path::PathBuf;

pub fn m17app_test() {
    //let path = PathBuf::from("../../../Data/test_vk7xt.rrc");
    let path = PathBuf::from("../../../Data/mymod.rrc");
    //let path = PathBuf::from("../../../Data/mymod-noisy.raw");
    let source = InputRrcFile::new(path);
    //let source = InputSoundcard::new();
    let soundmodem = Soundmodem::new(source, NullOutputSink::new(), NullPtt::new());
    let app = M17App::new(soundmodem);
    app.add_stream_adapter(Codec2Adapter::new());
    app.start();
    std::thread::sleep(std::time::Duration::from_secs(15));
}

fn main() {
    env_logger::init();
    m17app_test();
}
