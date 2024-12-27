use crate::tnc::Tnc;
use m17core::kiss::KissFrame;
use m17core::protocol::PacketType;
use m17core::traits::{PacketListener, StreamListener};

use log::debug;
use std::collections::HashMap;
use std::io::{Read, Write};
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

    pub fn add_packet_listener<P: PacketListener + 'static>(&self, listener: P) -> usize {
        let mut listeners = self.listeners.write().unwrap();
        let id = listeners.next;
        listeners.next += 1;
        listeners.packet.insert(id, Box::new(listener));
        id
    }

    pub fn add_stream_listener<S: StreamListener + 'static>(&self, listener: S) -> usize {
        let mut listeners = self.listeners.write().unwrap();
        let id = listeners.next;
        listeners.next += 1;
        listeners.stream.insert(id, Box::new(listener));
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

    // add more methods here for stream outgoing

    pub fn transmit_stream_start(&self /* lsf?, payload? what needs to be configured ?! */) {}

    // as long as there is only one TNC it is implied there is only ever one stream transmission in flight

    pub fn transmit_stream_next(&self, /* next payload,  */ end_of_stream: bool) {}

    pub fn start(&self) {
        let _ = self.event_tx.send(TncControlEvent::Start);
    }

    pub fn close(&self) {
        let _ = self.event_tx.send(TncControlEvent::Close);
    }
}

/// Synchronised structure for listeners subscribing to packets and streams.
///
/// Each listener will be notified in turn of each event.
struct Listeners {
    /// Identifier to be assigned to the next listener, starting from 0
    next: usize,
    packet: HashMap<usize, Box<dyn PacketListener>>,
    stream: HashMap<usize, Box<dyn StreamListener>>,
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
        let mut buf = [0u8; 1713];
        let mut n = 0;
        loop {
            // I want to call tnc.read() here
            // Probably these needs a helper in m17core::kiss? It will be common to both TNC and host

            // After a read...
            // if this does not start with FEND, forget all data up until first FEND
            // if we start with a FEND, see if there is another FEND with at least one byte between
            // for each such case, turn that FEND..=FEND slice into a KissFrame and attempt to parse it
            // once all such pairs have been handled...
            // move the last FEND onwards back to the start of the buffer
            //   - if there is no room to do so, this is an oversize frame. purge everything and carry on.
            // perform next read from end
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
