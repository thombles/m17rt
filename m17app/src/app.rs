use crate::adapter::{PacketAdapter, StreamAdapter};
use crate::link_setup::LinkSetup;
use crate::tnc::Tnc;
use m17core::kiss::{KissBuffer, KissCommand, KissFrame};
use m17core::protocol::{EncryptionType, LsfFrame, PacketType, StreamFrame};

use log::debug;
use std::collections::HashMap;
use std::sync::mpsc;
use std::sync::{Arc, RwLock};

pub struct M17App {
    adapters: Arc<RwLock<Adapters>>,
    event_tx: mpsc::SyncSender<TncControlEvent>,
}

impl M17App {
    pub fn new<T: Tnc + Send + 'static>(mut tnc: T) -> Self {
        let write_tnc = tnc.try_clone().unwrap();
        let (event_tx, event_rx) = mpsc::sync_channel(128);
        let listeners = Arc::new(RwLock::new(Adapters::new()));
        spawn_reader(tnc, listeners.clone());
        spawn_writer(write_tnc, event_rx);
        Self {
            adapters: listeners,
            event_tx,
        }
    }

    pub fn add_packet_adapter<P: PacketAdapter + 'static>(&self, adapter: P) -> usize {
        let adapter = Arc::new(adapter);
        let mut adapters = self.adapters.write().unwrap();
        let id = adapters.next;
        adapters.next += 1;
        adapters.packet.insert(id, adapter.clone());
        drop(adapters);
        adapter.adapter_registered(id, self.tx());
        id
    }

    pub fn add_stream_adapter<S: StreamAdapter + 'static>(&self, adapter: S) -> usize {
        let adapter = Arc::new(adapter);
        let mut adapters = self.adapters.write().unwrap();
        let id = adapters.next;
        adapters.next += 1;
        adapters.stream.insert(id, adapter.clone());
        drop(adapters);
        adapter.adapter_registered(id, self.tx());
        id
    }

    pub fn remove_packet_adapter(&self, id: usize) {
        if let Some(a) = self.adapters.write().unwrap().packet.remove(&id) {
            a.adapter_removed();
        }
    }

    pub fn remove_stream_adapter(&self, id: usize) {
        if let Some(a) = self.adapters.write().unwrap().stream.remove(&id) {
            a.adapter_removed();
        }
    }

    /// Create a handle that can be used to transmit data on the TNC
    pub fn tx(&self) -> TxHandle {
        TxHandle {
            event_tx: self.event_tx.clone(),
        }
    }

    pub fn start(&self) {
        let _ = self.event_tx.send(TncControlEvent::Start);
    }

    pub fn close(&self) {
        // TODO: blocking function to indicate TNC has finished closing
        // then we could call this in a signal handler to ensure PTT is dropped before quit
        let _ = self.event_tx.send(TncControlEvent::Close);
    }
}

pub struct TxHandle {
    event_tx: mpsc::SyncSender<TncControlEvent>,
}

impl TxHandle {
    pub fn transmit_packet(&self, link_setup: &LinkSetup, packet_type: PacketType, payload: &[u8]) {
        let (pack_type, pack_type_len) = packet_type.as_proto();
        if pack_type_len + payload.len() > 823 {
            // TODO: error for invalid transmission type
            return;
        }
        let mut full_payload = vec![];
        full_payload.extend_from_slice(&pack_type[0..pack_type_len]);
        full_payload.extend_from_slice(&payload);
        let crc = m17core::crc::m17_crc(&full_payload);
        full_payload.extend_from_slice(&crc.to_be_bytes());
        let kiss_frame = KissFrame::new_full_packet(&link_setup.raw.0, &full_payload).unwrap();
        let _ = self.event_tx.send(TncControlEvent::Kiss(kiss_frame));
    }

    pub fn transmit_stream_start(&self, link_setup: &LinkSetup) {
        let kiss_frame = KissFrame::new_stream_setup(&link_setup.raw.0).unwrap();
        let _ = self.event_tx.send(TncControlEvent::Kiss(kiss_frame));
    }

    // as long as there is only one TNC it is implied there is only ever one stream transmission in flight

    pub fn transmit_stream_next(&self, stream: &StreamFrame) {
        let kiss_frame = KissFrame::new_stream_data(&stream).unwrap();
        let _ = self.event_tx.send(TncControlEvent::Kiss(kiss_frame));
    }
}

/// Synchronised structure for listeners subscribing to packets and streams.
///
/// Each listener will be notified in turn of each event.
struct Adapters {
    /// Identifier to be assigned to the next listener, starting from 0
    next: usize,
    packet: HashMap<usize, Arc<dyn PacketAdapter>>,
    stream: HashMap<usize, Arc<dyn StreamAdapter>>,
}

impl Adapters {
    fn new() -> Self {
        Self {
            next: 0,
            packet: HashMap::new(),
            stream: HashMap::new(),
        }
    }
}

/// Carries a request from a method on M17App to the TNC's writer thread, which will execute it.
enum TncControlEvent {
    Kiss(KissFrame),
    Start,
    Close,
}

fn spawn_reader<T: Tnc>(mut tnc: T, adapters: Arc<RwLock<Adapters>>) {
    std::thread::spawn(move || {
        let mut kiss_buffer = KissBuffer::new();
        let mut stream_running = false;
        loop {
            let mut buf = kiss_buffer.buf_remaining();
            let n = match tnc.read(&mut buf) {
                Ok(n) => n,
                Err(_) => break,
            };
            kiss_buffer.did_write(n);
            while let Some(frame) = kiss_buffer.next_frame() {
                if frame.command() != Ok(KissCommand::DataFrame) {
                    continue;
                }
                match frame.port() {
                    Ok(m17core::kiss::PORT_PACKET_BASIC) => {
                        // no action
                        // we will handle the more full-featured version from from port 1
                    }
                    Ok(m17core::kiss::PORT_PACKET_FULL) => {
                        let mut payload = [0u8; 855]; // 30 byte LSF + 825 byte packet including CRC
                        let Ok(n) = frame.decode_payload(&mut payload) else {
                            debug!("failed to decode payload from KISS frame");
                            continue;
                        };
                        if n < 33 {
                            debug!("unusually short full packet frame");
                            continue;
                        }
                        let lsf = LsfFrame(payload[0..30].try_into().unwrap());
                        if lsf.check_crc() != 0 {
                            debug!("LSF in full packet frame did not pass CRC");
                            continue;
                        }
                        if lsf.encryption_type() != EncryptionType::None {
                            debug!("we only understand None encryption for now - skipping packet");
                            continue;
                        }
                        let Some((packet_type, type_len)) = PacketType::from_proto(&payload[30..n])
                        else {
                            debug!("failed to decode packet type");
                            continue;
                        };
                        if (n - 30 - type_len) < 2 {
                            debug!("packet payload too small to provide CRC");
                            continue;
                        }
                        let packet_crc = m17core::crc::m17_crc(&payload[30..n]);
                        if packet_crc != 0 {
                            debug!("packet CRC does not pass");
                            continue;
                        }
                        let packet_payload: Arc<[u8]> =
                            Arc::from(&payload[(30 + type_len)..(n - 2)]);

                        let subs: Vec<_> =
                            adapters.read().unwrap().packet.values().cloned().collect();
                        for s in subs {
                            s.packet_received(
                                LinkSetup::new_raw(lsf.clone()),
                                packet_type.clone(),
                                packet_payload.clone(),
                            );
                        }
                    }
                    Ok(m17core::kiss::PORT_STREAM) => {
                        let mut payload = [0u8; 32];
                        let Ok(n) = frame.decode_payload(&mut payload) else {
                            debug!("failed to decode stream payload from KISS frame");
                            continue;
                        };
                        if n == 30 {
                            let lsf = LsfFrame(payload[0..30].try_into().unwrap());
                            if lsf.check_crc() != 0 {
                                debug!("initial LSF in stream did not pass CRC");
                                continue;
                            }
                            stream_running = true;
                            let subs: Vec<_> =
                                adapters.read().unwrap().stream.values().cloned().collect();
                            for s in subs {
                                s.stream_began(LinkSetup::new_raw(lsf.clone()));
                            }
                        } else if n == 26 {
                            if !stream_running {
                                debug!("ignoring stream data as we didn't get a valid LSF first");
                                continue;
                            }
                            // TODO: parse LICH and handle the different changing subvalues META could have
                            if m17core::crc::m17_crc(&payload[6..n]) != 0 {
                                debug!("stream data CRC mismatch");
                                continue;
                            }
                            let mut frame_number = u16::from_be_bytes([payload[6], payload[7]]);
                            let is_final = (frame_number & 0x8000) > 0;
                            frame_number &= 0x7fff;
                            let data: [u8; 16] = payload[8..24].try_into().unwrap();
                            let data = Arc::new(data);
                            if is_final {
                                stream_running = false;
                            }
                            let subs: Vec<_> =
                                adapters.read().unwrap().stream.values().cloned().collect();
                            for s in subs {
                                s.stream_data(frame_number, is_final, data.clone());
                            }
                        }
                    }
                    _ => (),
                }
            }
        }
    });
}

fn spawn_writer<T: Tnc>(mut tnc: T, event_rx: mpsc::Receiver<TncControlEvent>) {
    std::thread::spawn(move || {
        while let Ok(ev) = event_rx.recv() {
            match ev {
                TncControlEvent::Kiss(k) => {
                    if let Err(e) = tnc.write_all(&k.as_bytes()) {
                        debug!("kiss send err: {:?}", e);
                        return;
                    }
                }
                TncControlEvent::Start => {
                    if let Err(e) = tnc.start() {
                        debug!("tnc start err: {:?}", e);
                        return;
                    }
                }
                TncControlEvent::Close => {
                    if let Err(e) = tnc.close() {
                        debug!("tnc close err: {:?}", e);
                        return;
                    }
                }
            }
        }
    });
}
