use std::{io::stdin, sync::Arc};

use clap::{Arg, value_parser};
use m17app::{
    adapter::StreamAdapter,
    app::M17App,
    link_setup::M17Address,
    reflector::{ReflectorClientConfig, ReflectorClientTnc, StatusHandler},
};
use m17codec2::{rx::Codec2RxAdapter, tx::Codec2TxAdapter};

fn main() {
    let args = clap::Command::new("m17rt-netclient")
        .arg(
            Arg::new("hostname")
                .long("hostname")
                .short('s')
                .required(true)
                .help("Domain or IP of reflector"),
        )
        .arg(
            Arg::new("port")
                .long("port")
                .short('p')
                .value_parser(value_parser!(u16))
                .default_value("17000")
                .help("Reflector listening port"),
        )
        .arg(
            Arg::new("callsign")
                .long("callsign")
                .short('c')
                .value_parser(valid_callsign)
                .required(true)
                .help("Your callsign for reflector registration and transmissions"),
        )
        .arg(
            Arg::new("module")
                .long("module")
                .short('m')
                .value_parser(valid_module)
                .required(true)
                .help("Module to connect to (A-Z)"),
        )
        .arg(
            Arg::new("input")
                .long("input")
                .short('i')
                .help("Soundcard name for microphone, otherwise system default"),
        )
        .arg(
            Arg::new("output")
                .long("output")
                .short('o')
                .help("Soundcard name for speaker, otherwise system default"),
        )
        .get_matches();

    let hostname = args.get_one::<String>("hostname").unwrap();
    let port = args.get_one::<u16>("port").unwrap();
    let callsign = args.get_one::<M17Address>("callsign").unwrap();
    let module = args.get_one::<char>("module").unwrap();
    let input = args.get_one::<String>("input");
    let output = args.get_one::<String>("output");

    let mut tx = Codec2TxAdapter::new(callsign.clone(), M17Address::new_broadcast());
    if let Some(input) = input {
        tx.set_input_card(input);
    }
    let ptt = tx.ptt();

    let mut rx = Codec2RxAdapter::new();
    if let Some(output) = output {
        rx.set_output_card(output);
    }

    let config = ReflectorClientConfig {
        hostname: hostname.clone(),
        port: *port,
        module: *module,
        local_callsign: callsign.clone(),
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
            "Transmission begins. From: {} To: {}",
            link_setup.source(),
            link_setup.destination()
        );
    }

    fn stream_data(&self, _frame_number: u16, is_final: bool, _data: Arc<[u8; 16]>) {
        if is_final {
            println!("Transmission ends.");
        }
    }
}

struct ConsoleStatusHandler;
impl StatusHandler for ConsoleStatusHandler {
    fn status_changed(&mut self, status: m17app::reflector::TncStatus) {
        println!("Client status: {status:?}")
    }
}
