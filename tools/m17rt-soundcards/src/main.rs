use m17app::soundcard::Soundcard;

fn main() {
    let inputs = Soundcard::supported_input_cards();
    let outputs = Soundcard::supported_output_cards();

    println!("\nSupported Input Soundcards ({}):\n", inputs.len());
    for i in inputs {
        println!("{}", i);
    }

    println!("\nSupported Output Soundcards ({}):\n", outputs.len());
    for o in outputs {
        println!("{}", o);
    }
}
