# m17codec2

Part of the [M17 Rust Toolkit](https://octet-stream.net/p/m17rt/). Pre-made adapters designed for the `m17app` crate that make it easier to work with Codec2 voice streams.

* `WavePlayer` - transmit a wave file as a stream (8 kHz, mono, 16 bit LE)
* `Codec2RxAdapter` - receive all incoming streams and attempt to play the decoded audio on a soundcard (configurable)
* `Codec2TxAdapter` - toggle a PTT to record audio from a microphone (soundcard also configurable), encode it, and transmit it with chosen source and destination addresses

**Important licence note:** While `m17codec2` is under the MIT licence, it uses the `codec2` crate as a dependency, which will statically link LGPL code in the build. If you are distributing software in a way where LGPL compliance requires special care (e.g., dynamic linking), consider implementing your own codec2 adapters in a way that is compliant in your scenario.
