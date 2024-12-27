==================
 M17 RUST TOOLKIT
==================

As of Dec 2024 this software is still under active development. The description that follows is not fully implemented yet.

M17RT is a collection of Rust crates and utilities to make it as easy to possible to implement programs that use the M17
Protocol for amateur radio: <https://m17foundation.org/projects/>.


                        ┌──────────────────────────────────────┐   <.
                   .>   │ m17app                               │    | Fan in data from adapters to TNC
 High level API    |    │ - High-level API for packets/streams │    | Fan out data from radio to adapters
 For PC-based apps |    │ - Sound card integration             │   <.
                   |    │ - TCP client/server KISS             │         vv KISS ^^
                   .>   │ - Multithreading                     │   <.
                        └──────────────────────────────────────┘    | Soundmodem worker thread:
                                                                    | Takes a sound card, PTT,
                        ┌──────────────────────────────────────┐    | assembles the components
                   .>   │ m17core                              │    | from m17core and puts it
 Low level API     |    │ - M17 KISS protocol                  │    | behind a KISS interface.
 no_std, no heap   |    │ - TNC / CSMA                         │    |
 could be no-float |    │ - M17 Data Link                      │    |
                   .>   │ - Baseband Modem                     │   <.
                        └──────────────────────────────────────┘


When you are writing an M17 packet or voice application you will target the high-level API and not concern yourself with
what kind of TNC will ultimately be used. It is modular - you could use a serial KISS modem, a TCP KISS service running
on another host or supplied by another program, or engage the built-in soundmodem by supplying a soundcard and PTT config.
This could be configured at runtime in your program.

Equally, the soundmodem can also be used as an independent module with any other M17 application that expects to speak to
a KISS TNC, including M17 applications that do not use this toolkit or are not written in Rust.

The basic structure of a program is that you will configure your TNC, use it to initialise an M17App, then add adapters to
the M17App which will handle all or a subset of the traffic. They will all share the same TNC.

Codec2 support follows the same pattern - the m17codec2 crate will provide standard M17App stream adapters to handle both:
    mic -> encode -> transmit stream
    incoming stream -> decode -> output on sound card

Splitting this into a separate crate serves two purposes. This reduces the dependency count if your app does not actually
use codec2. It also means you can avoid statically linking LGPL code into your Rust binary if you are relying on M17RT's
permissive licence. In this situation you can probably still find a way to use codec2 but it's not going to be as simple
as putting this in your Cargo.toml since Rust makes dynamic linking difficult.

Finally, there will be a series of utility binaries for modulation, demodulation, creating a KISS TCP server, etc. These
may be useful in their own right but their primary purpose is to test and demonstrate the toolkit. User-facing programs
should be their own projects that will provide proper attention to detail for their use cases.

=========
 LICENCE
=========

Copyright 2024 Thomas Karpiniec <tom.karpiniec@outlook.com>

M17 Rust Toolkit is made available under the MIT Licence. See LICENCE.TXT for details.
