use crate::adapter::{PacketAdapter, StreamAdapter};
use crate::error::{M17Error, M17Errors};
use crate::link_setup::LinkSetup;
use crate::tnc::Tnc;
use crate::{LsfFrame, PacketType, StreamFrame};
use m17core::kiss::{KissBuffer, KissCommand, KissFrame};
use m17core::protocol::EncryptionType;

use log::debug;
use std::collections::HashMap;
use std::sync::mpsc;
use std::sync::{Arc, RwLock};

#[derive(Debug, Clone, PartialEq, Eq, Copy)]
enum Lifecycle {
    Setup,
    Started,
    Closed,
}

pub struct M17App {
    adapters: Arc<RwLock<Adapters>>,
    event_tx: mpsc::SyncSender<TncControlEvent>,
    lifecycle: RwLock<Lifecycle>,
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
            lifecycle: RwLock::new(Lifecycle::Setup),
        }
    }

    pub fn add_packet_adapter<P: PacketAdapter + 'static>(
        &self,
        adapter: P,
    ) -> Result<usize, M17Error> {
        let adapter = Arc::new(adapter);
        let mut adapters = self.adapters.write().unwrap();
        let id = adapters.next;
        adapters.next += 1;
        adapters.packet.insert(id, adapter.clone());
        drop(adapters);
        if self.lifecycle() == Lifecycle::Started {
            adapter
                .start(self.tx())
                .map_err(|e| M17Error::Adapter(id, e))?;
        }
        Ok(id)
    }

    pub fn add_stream_adapter<S: StreamAdapter + 'static>(
        &self,
        adapter: S,
    ) -> Result<usize, M17Error> {
        let adapter = Arc::new(adapter);
        let mut adapters = self.adapters.write().unwrap();
        let id = adapters.next;
        adapters.next += 1;
        adapters.stream.insert(id, adapter.clone());
        drop(adapters);
        if self.lifecycle() == Lifecycle::Started {
            adapter
                .start(self.tx())
                .map_err(|e| M17Error::Adapter(id, e))?;
        }
        Ok(id)
    }

    pub fn remove_packet_adapter(&self, id: usize) -> Result<(), M17Error> {
        if let Some(a) = self.adapters.write().unwrap().packet.remove(&id) {
            if self.lifecycle() == Lifecycle::Started {
                a.close().map_err(|e| M17Error::Adapter(id, e))?;
            }
        }
        Ok(())
    }

    pub fn remove_stream_adapter(&self, id: usize) -> Result<(), M17Error> {
        if let Some(a) = self.adapters.write().unwrap().stream.remove(&id) {
            if self.lifecycle() == Lifecycle::Started {
                a.close().map_err(|e| M17Error::Adapter(id, e))?;
            }
        }
        Ok(())
    }

    /// Create a handle that can be used to transmit data on the TNC
    pub fn tx(&self) -> TxHandle {
        TxHandle {
            event_tx: self.event_tx.clone(),
        }
    }

    pub fn start(&self) -> Result<(), M17Errors> {
        if self.lifecycle() != Lifecycle::Setup {
            return Err(M17Errors(vec![M17Error::InvalidStart]));
        }
        self.set_lifecycle(Lifecycle::Started);
        let mut errs = vec![];
        {
            let adapters = self.adapters.read().unwrap();
            for (i, p) in &adapters.packet {
                if let Err(e) = p.start(self.tx()) {
                    errs.push(M17Error::Adapter(*i, e));
                }
            }
            for (i, s) in &adapters.stream {
                if let Err(e) = s.start(self.tx()) {
                    errs.push(M17Error::Adapter(*i, e));
                }
            }
        }
        let _ = self.event_tx.send(TncControlEvent::Start);
        if errs.is_empty() {
            Ok(())
        } else {
            Err(M17Errors(errs))
        }
    }

    pub fn close(&self) -> Result<(), M17Errors> {
        if self.lifecycle() != Lifecycle::Started {
            return Err(M17Errors(vec![M17Error::InvalidClose]));
        }
        self.set_lifecycle(Lifecycle::Closed);
        let mut errs = vec![];
        {
            let adapters = self.adapters.read().unwrap();
            for (i, p) in &adapters.packet {
                if let Err(e) = p.close() {
                    errs.push(M17Error::Adapter(*i, e));
                }
            }
            for (i, s) in &adapters.stream {
                if let Err(e) = s.close() {
                    errs.push(M17Error::Adapter(*i, e));
                }
            }
        }
        // TODO: blocking function to indicate TNC has finished closing
        // then we could call this in a signal handler to ensure PTT is dropped before quit
        let _ = self.event_tx.send(TncControlEvent::Close);
        if errs.is_empty() {
            Ok(())
        } else {
            Err(M17Errors(errs))
        }
    }

    fn lifecycle(&self) -> Lifecycle {
        *self.lifecycle.read().unwrap()
    }

    fn set_lifecycle(&self, lifecycle: Lifecycle) {
        *self.lifecycle.write().unwrap() = lifecycle;
    }
}

pub struct TxHandle {
    event_tx: mpsc::SyncSender<TncControlEvent>,
}

impl TxHandle {
    pub fn transmit_packet(
        &self,
        link_setup: &LinkSetup,
        packet_type: PacketType,
        payload: &[u8],
    ) -> Result<(), M17Error> {
        let (pack_type, pack_type_len) = packet_type.as_proto();
        if pack_type_len + payload.len() > 823 {
            return Err(M17Error::PacketTooLarge {
                provided: payload.len(),
                capacity: 823 - pack_type_len,
            });
        }
        let mut full_payload = vec![];
        full_payload.extend_from_slice(&pack_type[0..pack_type_len]);
        full_payload.extend_from_slice(payload);
        let crc = m17core::crc::m17_crc(&full_payload);
        full_payload.extend_from_slice(&crc.to_be_bytes());
        let kiss_frame = KissFrame::new_full_packet(&link_setup.raw.0, &full_payload).unwrap();
        let _ = self.event_tx.send(TncControlEvent::Kiss(kiss_frame));
        Ok(())
    }

    pub fn transmit_stream_start(&self, link_setup: &LinkSetup) {
        let kiss_frame = KissFrame::new_stream_setup(&link_setup.raw.0).unwrap();
        let _ = self.event_tx.send(TncControlEvent::Kiss(kiss_frame));
    }

    // as long as there is only one TNC it is implied there is only ever one stream transmission in flight

    pub fn transmit_stream_next(&self, stream: &StreamFrame) {
        let kiss_frame = KissFrame::new_stream_data(stream).unwrap();
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
#[allow(clippy::large_enum_variant)]
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
            let buf = kiss_buffer.buf_remaining();
            let n = match tnc.read(buf) {
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
                                packet_type,
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
                    if let Err(e) = tnc.write_all(k.as_bytes()) {
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

#[cfg(test)]
mod tests {
    use crate::error::AdapterError;
    use crate::{link_setup::M17Address, test_util::NullTnc};

    use super::*;

    #[test]
    fn packet_payload_len() {
        let app = M17App::new(NullTnc);
        let res = app.tx().transmit_packet(
            &LinkSetup::new_packet(&M17Address::new_broadcast(), &M17Address::new_broadcast()),
            PacketType::Raw,
            &[0u8; 100],
        );
        assert!(matches!(res, Ok(())));
        let res = app.tx().transmit_packet(
            &LinkSetup::new_packet(&M17Address::new_broadcast(), &M17Address::new_broadcast()),
            PacketType::Raw,
            &[0u8; 900],
        );
        assert!(matches!(
            res,
            Err(M17Error::PacketTooLarge {
                provided: 900,
                capacity: 822
            })
        ));
    }

    #[test]
    fn adapter_lifecycle() {
        #[derive(Debug, PartialEq)]
        enum Event {
            Started,
            Closed,
        }
        macro_rules! event_impl {
            ($target:ty, $trait:ty) => {
                impl $trait for $target {
                    fn start(&self, _handle: TxHandle) -> Result<(), AdapterError> {
                        self.0.send(Event::Started)?;
                        Ok(())
                    }

                    fn close(&self) -> Result<(), AdapterError> {
                        self.0.send(Event::Closed)?;
                        Ok(())
                    }
                }
            };
        }
        struct FakePacket(mpsc::SyncSender<Event>);
        struct FakeStream(mpsc::SyncSender<Event>);
        event_impl!(FakePacket, PacketAdapter);
        event_impl!(FakeStream, StreamAdapter);

        let app = M17App::new(NullTnc);
        let (tx_p, rx_p) = mpsc::sync_channel(128);
        let (tx_s, rx_s) = mpsc::sync_channel(128);
        let packet = FakePacket(tx_p);
        let stream = FakeStream(tx_s);

        let id_p = app.add_packet_adapter(packet).unwrap();
        let id_s = app.add_stream_adapter(stream).unwrap();
        app.start().unwrap();
        app.close().unwrap();
        app.remove_packet_adapter(id_p).unwrap();
        app.remove_stream_adapter(id_s).unwrap();

        assert_eq!(rx_p.try_recv(), Ok(Event::Started));
        assert_eq!(rx_p.try_recv(), Ok(Event::Closed));
        assert_eq!(rx_p.try_recv(), Err(mpsc::TryRecvError::Disconnected));

        assert_eq!(rx_s.try_recv(), Ok(Event::Started));
        assert_eq!(rx_s.try_recv(), Ok(Event::Closed));
        assert_eq!(rx_s.try_recv(), Err(mpsc::TryRecvError::Disconnected));
    }
}
