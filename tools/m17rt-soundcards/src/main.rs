use ascii_table::{Align, AsciiTable};
use m17app::soundcard::Soundcard;
use m17codec2::{rx::Codec2RxAdapter, tx::Codec2TxAdapter};

fn main() {
    // On some platforms enumerating devices will emit junk to the terminal:
    // https://github.com/RustAudio/cpal/issues/384
    // To minimise the impact, enumerate first and put our output at the end.
    let soundmodem_in = Soundcard::supported_input_cards();
    let soundmodem_out = Soundcard::supported_output_cards();
    let codec2_in = Codec2TxAdapter::supported_input_cards();
    let codec2_out = Codec2RxAdapter::supported_output_cards();

    println!("\nDetected sound cards compatible with M17 Rust Toolkit:");

    generate_table(
        "SOUNDMODEM",
        "INPUT",
        "OUTPUT",
        &soundmodem_in,
        &soundmodem_out,
    );
    generate_table("CODEC2 AUDIO", "TX", "RX", &codec2_in, &codec2_out);
}

fn generate_table(
    heading: &str,
    input: &str,
    output: &str,
    input_cards: &[String],
    output_cards: &[String],
) {
    let mut merged: Vec<&str> = input_cards
        .iter()
        .chain(output_cards.iter())
        .map(|s| s.as_str())
        .collect();
    merged.sort();
    merged.dedup();
    let yes = "OK";
    let no = "";
    let data = merged.into_iter().map(|c| {
        [
            c,
            if input_cards.iter().any(|s| s == c) {
                yes
            } else {
                no
            },
            if output_cards.iter().any(|s| s == c) {
                yes
            } else {
                no
            },
        ]
    });

    let mut table = AsciiTable::default();
    table.column(0).set_header(heading).set_align(Align::Left);
    table.column(1).set_header(input).set_align(Align::Center);
    table.column(2).set_header(output).set_align(Align::Center);
    table.print(data);
}
