use crate::adapter::{PacketAdapter, StreamAdapter};
use crate::tnc::Tnc;
use m17core::kiss::{KissBuffer, KissCommand, KissFrame};
use m17core::protocol::{EncryptionType, LsfFrame, PacketType};

use log::debug;
use std::collections::HashMap;
use std::sync::mpsc;
use std::sync::{Arc, RwLock};

pub struct M17App {
    listeners: Arc<RwLock<Listeners>>,
    event_tx: mpsc::SyncSender<TncControlEvent>,
}

impl M17App {
    pub fn new<T: Tnc + Send + 'static>(mut tnc: T) -> Self {
        let write_tnc = tnc.try_clone().unwrap();
        let (event_tx, event_rx) = mpsc::sync_channel(128);
        let listeners = Arc::new(RwLock::new(Listeners::new()));
        spawn_reader(tnc, listeners.clone());
        spawn_writer(write_tnc, event_rx);
        Self {
            listeners,
            event_tx,
        }
    }

    pub fn add_packet_listener<P: PacketAdapter + 'static>(&self, listener: P) -> usize {
        let mut listeners = self.listeners.write().unwrap();
        let id = listeners.next;
        listeners.next += 1;
        listeners.packet.insert(id, Arc::new(listener));
        id
    }

    pub fn add_stream_listener<S: StreamAdapter + 'static>(&self, listener: S) -> usize {
        let mut listeners = self.listeners.write().unwrap();
        let id = listeners.next;
        listeners.next += 1;
        listeners.stream.insert(id, Arc::new(listener));
        id
    }

    pub fn remove_packet_listener(&self, id: usize) {
        self.listeners.write().unwrap().packet.remove(&id);
    }

    pub fn remove_stream_listener(&self, id: usize) {
        self.listeners.write().unwrap().stream.remove(&id);
    }

    pub fn transmit_packet(&self, type_code: PacketType, payload: &[u8]) {
        // hang on where do we get the LSF details from? We need a destination obviously
        // our source address needs to be configured here too
        // also there is possible CAN, encryption, meta payload

        // we will immediately convert this into a KISS payload before sending into channel so we only need borrow on data
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
        let _ = self.event_tx.send(TncControlEvent::Close);
    }
}

pub struct TxHandle {
    event_tx: mpsc::SyncSender<TncControlEvent>,
}

impl TxHandle {
    // add more methods here for stream outgoing

    pub fn transmit_stream_start(&self /* lsf?, payload? what needs to be configured ?! */) {}

    // as long as there is only one TNC it is implied there is only ever one stream transmission in flight

    pub fn transmit_stream_next(&self, /* next payload,  */ end_of_stream: bool) {}
}

/// Synchronised structure for listeners subscribing to packets and streams.
///
/// Each listener will be notified in turn of each event.
struct Listeners {
    /// Identifier to be assigned to the next listener, starting from 0
    next: usize,
    packet: HashMap<usize, Arc<dyn PacketAdapter>>,
    stream: HashMap<usize, Arc<dyn StreamAdapter>>,
}

impl Listeners {
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

fn spawn_reader<T: Tnc + Send + 'static>(mut tnc: T, listeners: Arc<RwLock<Listeners>>) {
    std::thread::spawn(move || {
        let mut kiss_buffer = KissBuffer::new();
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
                        if lsf.crc() != 0 {
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
                            listeners.read().unwrap().packet.values().cloned().collect();
                        for s in subs {
                            s.packet_received(
                                lsf.clone(),
                                packet_type.clone(),
                                packet_payload.clone(),
                            );
                        }
                    }
                    Ok(m17core::kiss::PORT_STREAM) => {
                        // handle stream and send it to subscribers
                    }
                    _ => (),
                }
            }
        }
    });
}

fn spawn_writer<T: Tnc + Send + 'static>(mut tnc: T, event_rx: mpsc::Receiver<TncControlEvent>) {
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
