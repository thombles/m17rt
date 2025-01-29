# m17codec2

Part of the [M17 Rust Toolkit](https://octet-stream.net/p/m17rt/).

This crate includes a modulator, demodulator, TNC, M17 data link parsing and encoding, KISS protocol handling, and other protocol utilities. It can be used to create an M17 transmitter or receiver, however you will have to connect everything together yourself. If possible, consider using the higher-level crate `m17app`.

`m17core` is `no_std`, does not perform any heap allocations, and its protocol implementations are non-blocking and sans-I/O.

You might be interested in using this crate directly for:

* Developing on bare metal targets where `std` is not available or appropriate
* Specialised M17 utilities or simulations

There is an implied protocol between `SoftModulator`, `SoftDemodulator` and `SoftTnc`. For a full example see the implementation of `Soundmodem` in `m17app`.

In brief: the rx path is that new samples will be given to `SoftDemodulator`. It may emit a frame, which should be delivered to `SoftTnc`. In turn, it may emit a KISS frame for the host.

The tx path is a little more complicated. You must supply a ring buffer which is shared between the DAC consuming samples and the `SoftModulator` creating samples. The `SoftTnc` indicates when a transmission begins, then the flow of data is controlled by `SoftModulator` which will opportunistically draw new frames out of the TNC to keep the output buffer topped up. When the TNC indicates the end of the transmission, it will wait for the `SoftModulator` to indicate when tx will finish and PTT should be disengaged. While this is occurring, new stream frames should be delivered via `SoftTnc`'s KISS interface at an equal ratio to the output samples being read so that buffers do not overflow or underrun.

