# M17 Rust Toolkit

**`m17app`** - [crates.io](https://crates.io/crates/m17app), [API Reference](https://docs.rs/m17app/)  
**`m17core`** - [crates.io](https://crates.io/crates/m17core), [API Reference](https://docs.rs/m17core/)  
**`m17codec2`** - [crates.io](https://crates.io/crates/m17codec2), [API Reference](https://docs.rs/m17codec2/)

The **M17 Rust Toolkit** is a collection of Rust libraries and utilities for building software and experimenting with the [M17 digital radio protocol](https://m17project.org/). The goal is that it is straightforward to build PC-based M17 applications, while retaining flexibility for custom scenarios or more constrained platforms.

The recommended starting point is the crate **`m17app`**, which provides a high-level API for application developers. It is designed for building radio software that runs on regular PCs or equivalently powerful devices like a smartphone or Raspberry Pi. You can either point it at an external TNC or activate the built-in soundmodem to use a standard soundcard and serial PTT.

![Image](https://github.com/user-attachments/assets/fa17e347-57d1-4de6-9c00-6eb4ca69d4fc)

`m17app` can be considered an easy-to-use wrapper around `m17core`, a separate crate which provides all the modem and TNC functions.

Let's see how to use `m17app` in practice. If you prefer full examples, see [sending a packet](https://github.com/thombles/m17rt/blob/master/tools/m17rt-txpacket/src/main.rs) and [receiving a packet](https://github.com/thombles/m17rt/blob/master/tools/m17rt-rxpacket/src/main.rs).

## Creating an `M17App`

The most important type is `M17App`. This is what your program can use to transmit packets and streams, or to subscribe to incoming packets and streams. To create an `M17App` you must provide it with a TNC, which is any type that implements the trait `Tnc`. This could be a `TcpStream` to another TNC device exposed to the network or it could be an instance of the built-in `Soundmodem`.

## Creating a `Soundmodem`

A `Soundmodem` can use soundcards in your computer to send and receive M17 baseband signals via a radio. More generally it can accept input samples from any compatible source, and provide output samples to any compatible sink, and it will coordinate the modem and TNC in realtime on a background thread.

A `Soundmodem` requires three parameters:

* _Input source_ - the signal we are receiving
* _Output sink_ - somewhere to send the modulated signal we want to transmit
* _PTT_ - a transmit switch that can be turned on or off

These are all traits that you can implement yourself but you can probably use one of the types already included in `m17app`.

Provided inputs:

* `Soundcard` - Once you have initialised a card, call `input()` to get an input source handle to provide to the `Soundmodem`.
* `RtlSdr` - Receive using an RTL-SDR dongle. This requires that the `rtl_fm` utility is installed and present in your path.
* `InputRrcFile` - Read from an M17 `.rrc` file on disk, which contains shaped baseband data as 16-bit LE 48 kHz samples.
* `NullInputSource` - Fake device that provides a continuous stream of silence.

Provided outputs:

* `Soundcard` - Once you have initialised a card, call `output()` to get an output sink handle.
* `OutputRrcFile` - Write transmissions to a `.rrc` on disk.
* `NullOutputSink` - Fake device that will swallow any samples it is given.

Provided PTTs:

* `SerialPtt` - Use a serial/COM port with either the RTS or DTR pin to activate PTT.
* `NullPtt` - Fake device that will not control any real PTT.

For `Soundcard` you will need to identify the soundcard by a string name. The format of this card name is specific to the audio library used (`cpal`). Use `Soundcard::supported_input_cards()` and `Soundcard::supported_output_cards()` to list compatible devices. The bundled utility `m17rt-soundcards` may be useful. Similarly, `SerialPtt::available_ports()` lists the available serial ports.

If you're using a Digirig on a Linux PC, M17 setup might look like this:

```rust
    let soundcard = Soundcard::new("plughw:CARD=Device,DEV=0").unwrap();
    let ptt = SerialPtt::new("/dev/ttyUSB0", PttPin::Rts);
    let soundmodem = Soundmodem::new(soundcard.input(), soundcard.output(), ptt);
    let app = M17App::new(soundmodem);
    app.start();
```

## Working with packets

First let's transmit a packet. We will need to configure some metadata for the transmission, beginning with the source and destination callsigns. Create suitable addresses of type `M17Address`, which will validate that the address is a valid format.

```rust
    let source = M17Address::from_callsign("VK7XT-1").unwrap();
    let destination = M17Address::new_broadcast();
```

All M17 transmissions require a link setup frame which includes the source and destination addresses plus other data. If you wish, you can use the raw `LsfFrame` type to create exactly the frame you want. Here we will use a convenience method to create an LSF for unencrypted packet data.

```rust
    let link_setup = LinkSetup::new_packet(&source, &destination);
```

Transmissions are made via a `TxHandle`, which you can create by calling `app.tx()`. We must provide the packet application type and the payload as a byte slice, up to approx 822 bytes. This sends the transmission command to the TNC, which will transmit it when the channel is clear.

```rust
    let payload = b"Hello, world!";
    app.tx()
        .transmit_packet(&link_setup, PacketType::Sms, payload);
```

Next let's see how to receive a packet. To subscribe to incoming packets you need to provide a subscriber that implements the trait `PacketAdapter`. This includes a number of lifecycle methods which are optional to implement. In this case we will handle `packet_received` and print a summary of the received packet and its contents to stdout.

```rust
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
```

We instantiate one of these subscribers and provide it to our instance of `M17App`.

```rust
    app.add_packet_adapter(PacketPrinter);
```

Note that if the adapter also implemented `adapter_registered`, then it would receive a copy of `TxHandle`. This allows you to create self-contained adapter implementations that can both transmit and receive.

Adding an adapter returns an identifier that you can use it to remove it again later if you wish. You can add an arbitrary number of adapters. Each will receive its own copy of the packet (or stream, as in the next section).

## Working with streams

M17 also provides streams, which are continuous transmissions of arbitrary length. Unlike packets, you are not guaranteed to receive every frame, and it is possible for a receiver to lock on to a transmission that has previously started and begin decoding it in the middle. These streams may contain voice (generally 3200 bit/s Codec2), arbitrary data, or a combination of voice and data.

For our first example, let's see how to use the `m17codec2` helper crate to send and receive Codec2 audio.

The following line will register an adapter that monitors incoming M17 streams, attempts to decode the Codec2, and play the decoded audio on the default system sound card.

```rust
    app.add_stream_adapter(Codec2Adapter::new());
```

This is how to transmit a wave file of human speech (8 kHz, mono, 16 bit LE) as a Codec2 stream:

```rust
    WavePlayer::play(
        PathBuf::from("audio.wav"),
        app.tx(),                                       // TxHandle
        &M17Address::from_callsign("VK7XT-1").unwrap(), // source
        &M17Address::new_broadcast(),                   // destination
        0,                                              // channel access number
    );
```

Transmitting and receiving your own stream types works in a similar way to packets however the requirements are somewhat stricter.

To transmit:

1. Construct a LinkSetup frame, possibly using the `LinkSetup::new_voice()` helper, and call `tx.transmit_stream_start(lsf)`
2. Immediately construct a `StreamFrame` with data and call `tx.transmit_stream_next(stream_frame)`
3. Continue sending a `StreamFrame` every 40 ms until you finish with one where `end_of_stream` is set to `true`.

You are required to fill in two LICH-related fields in `StreamFrame` yourself. The counter should rotate from 0 to 5 (inclusive), and you can get the corresponding bytes using the `lich_part()` helper method on your original `LinkSetup`. The frame number starts at 0 and counts upward.

To receive:

1. Create an adapter that implements trait `StreamAdapter`
2. Handle the `stream_began` and `stream_data` methods
3. Add it to your `M17App`

## Licence

Copyright 2025 Thomas Karpiniec.

M17 Rust Toolkit is made available under the MIT Licence. See LICENCE.TXT for details.
