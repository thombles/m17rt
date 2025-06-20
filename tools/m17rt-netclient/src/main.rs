use std::{io::stdin, sync::Arc};

use clap::Parser;
use m17app::{
    adapter::StreamAdapter,
    app::M17App,
    link_setup::M17Address,
    reflector::{ReflectorClientConfig, ReflectorClientTnc, StatusHandler},
};
use m17codec2::{rx::Codec2RxAdapter, tx::Codec2TxAdapter};

#[derive(Parser)]
struct Args {
    #[arg(short = 's', help = "Domain or IP of reflector")]
    hostname: String,
    #[arg(
        short = 'p',
        default_value = "17000",
        help = "Reflector listening port"
    )]
    port: u16,
    #[arg(short = 'c', value_parser = valid_callsign, help = "Your callsign for reflector registration and transmissions")]
    callsign: M17Address,
    #[arg(short = 'r', value_parser = valid_callsign, help = "Reflector designator/callsign, often starting with 'M17-'")]
    reflector: M17Address,
    #[arg(short = 'm', value_parser = valid_module, help = "Module to connect to (A-Z)")]
    module: char,
    #[arg(
        short = 'i',
        help = "Soundcard name for microphone, otherwise system default"
    )]
    input: Option<String>,
    #[arg(
        short = 'o',
        help = "Soundcard name for speaker, otherwise system default"
    )]
    output: Option<String>,
}

fn main() {
    let args = Args::parse();

    // It is current convention that mrefd requires the destination of transmissions to match the reflector.
    // If you are connected to "M17-XXX" on module B then you must set the dst to "M17-XXX B".
    // This requirement is likely to change but for the purposes of this test client we'll hard-code the
    // behaviour for the time being.
    let ref_with_mod = format!("{} {}", args.reflector, args.module);
    let Ok(reflector) = M17Address::from_callsign(&ref_with_mod) else {
        println!(
            "Unable to create valid destination address for reflector + callsign '{ref_with_mod}'"
        );
        std::process::exit(1);
    };

    let mut tx = Codec2TxAdapter::new(args.callsign.clone(), reflector);
    if let Some(input) = args.input {
        tx.set_input_card(input);
    }
    let ptt = tx.ptt();

    let mut rx = Codec2RxAdapter::new();
    if let Some(output) = args.output {
        rx.set_output_card(output);
    }

    let config = ReflectorClientConfig {
        hostname: args.hostname,
        port: args.port,
        module: args.module,
        local_callsign: args.callsign,
    };
    let tnc = ReflectorClientTnc::new(config, ConsoleStatusHandler);
    let app = M17App::new(tnc);
    app.add_stream_adapter(ConsoleAdapter).unwrap();
    app.add_stream_adapter(tx).unwrap();
    app.add_stream_adapter(rx).unwrap();
    app.start().unwrap();

    println!(">>> PRESS ENTER TO TOGGLE PTT <<<");
    let mut buf = String::new();

    loop {
        let _ = stdin().read_line(&mut buf);
        ptt.set_ptt(true);
        println!("PTT ON: PRESS ENTER TO END");

        let _ = stdin().read_line(&mut buf);
        ptt.set_ptt(false);
        println!("PTT OFF");
    }
}

fn valid_module(m: &str) -> Result<char, String> {
    let m = m.to_ascii_uppercase();
    if m.len() != 1 || !m.chars().next().unwrap().is_alphabetic() {
        return Err("Module must be a single letter from A to Z".to_owned());
    }
    Ok(m.chars().next().unwrap())
}

fn valid_callsign(c: &str) -> Result<M17Address, String> {
    M17Address::from_callsign(c).map_err(|e| e.to_string())
}

struct ConsoleAdapter;
impl StreamAdapter for ConsoleAdapter {
    fn stream_began(&self, link_setup: m17app::link_setup::LinkSetup) {
        println!(
            "Incoming transmission begins. From: {} To: {}",
            link_setup.source(),
            link_setup.destination()
        );
    }

    fn stream_data(&self, _frame_number: u16, is_final: bool, _data: Arc<[u8; 16]>) {
        if is_final {
            println!("Incoming transmission ends.");
        }
    }
}

struct ConsoleStatusHandler;
impl StatusHandler for ConsoleStatusHandler {
    fn status_changed(&mut self, status: m17app::reflector::TncStatus) {
        println!("Client status: {status:?}")
    }
}
