use std::{
    io::{self, Read, Write},
    net::{Ipv4Addr, Ipv6Addr, SocketAddr, ToSocketAddrs, UdpSocket},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender},
        Arc, Mutex,
    },
    thread,
    time::Duration,
};

use crate::{link_setup::M17Address, tnc::Tnc, util::out_buffer::OutBuffer};
use m17core::{
    kiss::{KissBuffer, KissCommand, KissFrame, PORT_STREAM},
    protocol::{LsfFrame, StreamFrame},
    reflector::{
        convert::{RfToVoice, VoiceToRf},
        packet::{Connect, Pong, ServerMessage, Voice},
    },
};

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct ReflectorClientConfig {
    hostname: String,
    port: u16,
    module: char,
    local_callsign: M17Address,
}

type WrappedStatusHandler = Arc<Mutex<dyn StatusHandler + Send + 'static>>;

/// Network-based TNC that attempts to maintain a UDP connection to a reflector.
///
/// Streams will be sent and received over IP rather than RF.
#[derive(Clone)]
pub struct ReflectorClientTnc {
    config: ReflectorClientConfig,
    status_handler: WrappedStatusHandler,
    kiss_out_tx: Sender<Arc<[u8]>>,
    kiss_out: OutBuffer,
    event_tx: Arc<Mutex<Option<Sender<TncEvent>>>>,
    is_closed: Arc<AtomicBool>,
    kiss_buffer: Arc<Mutex<KissBuffer>>,
    rf_to_voice: Arc<Mutex<Option<RfToVoice>>>,
}

impl ReflectorClientTnc {
    /// Create a new Reflector Client TNC.
    ///
    /// You must provide a configuration object and a handler for status events, such as when the TNC
    /// connects and disconnects. The status events are purely information and if you're not interested
    /// in them, provide a `NullStatusHandler`.
    pub fn new<S: StatusHandler + Send + 'static>(
        config: ReflectorClientConfig,
        status: S,
    ) -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            config,
            status_handler: Arc::new(Mutex::new(status)),
            kiss_out_tx: tx,
            kiss_out: OutBuffer::new(rx),
            event_tx: Arc::new(Mutex::new(None)),
            is_closed: Arc::new(AtomicBool::new(false)),
            kiss_buffer: Arc::new(Mutex::new(KissBuffer::new())),
            rf_to_voice: Arc::new(Mutex::new(None)),
        }
    }
}

impl Read for ReflectorClientTnc {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.kiss_out.read(buf)
    }
}

impl Write for ReflectorClientTnc {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let mut kiss = self.kiss_buffer.lock().unwrap();
        let rem = kiss.buf_remaining();
        let sz = buf.len().max(rem.len());
        rem[0..sz].copy_from_slice(&buf[0..sz]);
        if let Some(frame) = kiss.next_frame() {
            if Ok(KissCommand::DataFrame) == frame.command() && frame.port() == Ok(PORT_STREAM) {
                let mut payload = [0u8; 30];
                if let Ok(len) = frame.decode_payload(&mut payload) {
                    if len == 30 {
                        let lsf = LsfFrame(payload);
                        let mut to_voice = self.rf_to_voice.lock().unwrap();
                        match &mut *to_voice {
                            Some(to_voice) => to_voice.process_lsf(lsf),
                            None => *to_voice = Some(RfToVoice::new(lsf)),
                        }
                    } else if len == 26 {
                        let frame_num_part = u16::from_be_bytes([payload[6], payload[7]]);
                        let frame = StreamFrame {
                            lich_idx: payload[5] >> 5,
                            lich_part: payload[0..5].try_into().unwrap(),
                            frame_number: frame_num_part & 0x7fff,
                            end_of_stream: frame_num_part & 0x8000 > 0,
                            stream_data: payload[8..24].try_into().unwrap(),
                        };
                        let to_voice = self.rf_to_voice.lock().unwrap();
                        if let Some(to_voice) = &*to_voice {
                            let voice = to_voice.process_stream(&frame);
                            if let Some(tx) = self.event_tx.lock().unwrap().as_ref() {
                                let _ = tx.send(TncEvent::TransmitVoice(voice));
                            }
                        }
                    }
                };
            }
        }
        Ok(sz)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Tnc for ReflectorClientTnc {
    fn try_clone(&mut self) -> Result<Self, crate::tnc::TncError> {
        Ok(self.clone())
    }

    fn start(&mut self) {
        spawn_runner(
            self.config.clone(),
            self.status_handler.clone(),
            self.event_tx.clone(),
            self.is_closed.clone(),
            self.kiss_out_tx.clone(),
        );
    }

    fn close(&mut self) {
        if let Some(tx) = self.event_tx.lock().unwrap().as_ref() {
            self.is_closed.store(true, Ordering::Release);
            let _ = tx.send(TncEvent::Close);
        }
    }
}

#[allow(clippy::large_enum_variant)]
enum TncEvent {
    Close,
    Received(ServerMessage),
    TransmitVoice(Voice),
}

fn spawn_runner(
    config: ReflectorClientConfig,
    status: WrappedStatusHandler,
    event_tx: Arc<Mutex<Option<Sender<TncEvent>>>>,
    is_closed: Arc<AtomicBool>,
    kiss_out_tx: Sender<Arc<[u8]>>,
) {
    std::thread::spawn(move || {
        status
            .lock()
            .unwrap()
            .status_changed(TncStatus::Disconnected);
        while !is_closed.load(Ordering::Acquire) {
            status.lock().unwrap().status_changed(TncStatus::Connecting);
            let sa = if let Ok(mut sa_iter) =
                (config.hostname.as_str(), config.port).to_socket_addrs()
            {
                if let Some(sa) = sa_iter.next() {
                    sa
                } else {
                    status
                        .lock()
                        .unwrap()
                        .status_changed(TncStatus::Disconnected);
                    thread::sleep(Duration::from_secs(10));
                    continue;
                }
            } else {
                status
                    .lock()
                    .unwrap()
                    .status_changed(TncStatus::Disconnected);
                thread::sleep(Duration::from_secs(10));
                continue;
            };
            let (tx, rx) = mpsc::channel();
            *event_tx.lock().unwrap() = Some(tx.clone());
            if !is_closed.load(Ordering::Acquire) {
                run_single_conn(
                    sa,
                    tx,
                    rx,
                    kiss_out_tx.clone(),
                    config.clone(),
                    status.clone(),
                );
            }
        }
        status.lock().unwrap().status_changed(TncStatus::Closed);
    });
}

fn run_single_conn(
    dest: SocketAddr,
    event_tx: Sender<TncEvent>,
    event_rx: Receiver<TncEvent>,
    kiss_out_tx: Sender<Arc<[u8]>>,
    config: ReflectorClientConfig,
    status: WrappedStatusHandler,
) {
    let socket = if dest.is_ipv4() {
        UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).unwrap()
    } else {
        UdpSocket::bind((Ipv6Addr::UNSPECIFIED, 0)).unwrap()
    };

    let mut connect = Connect::new();
    connect.set_address(config.local_callsign.address().to_owned());
    connect.set_module(config.module);
    let _ = socket.send_to(connect.as_bytes(), dest);
    let mut converter = VoiceToRf::new();
    let single_conn_ended = Arc::new(AtomicBool::new(false));
    // TODO: unwrap
    spawn_reader(
        socket.try_clone().unwrap(),
        event_tx,
        single_conn_ended.clone(),
    );

    while let Ok(ev) = event_rx.recv_timeout(Duration::from_secs(30)) {
        match ev {
            TncEvent::Close => {
                break;
            }
            TncEvent::Received(server_msg) => match server_msg {
                ServerMessage::ConnectAcknowledge(_) => {
                    status.lock().unwrap().status_changed(TncStatus::Connected);
                }
                ServerMessage::ConnectNack(_) => {
                    status
                        .lock()
                        .unwrap()
                        .status_changed(TncStatus::ConnectRejected);
                    break;
                }
                ServerMessage::ForceDisconnect(_) => {
                    status
                        .lock()
                        .unwrap()
                        .status_changed(TncStatus::ForceDisconnect);
                    break;
                }
                ServerMessage::Voice(voice) => {
                    let (lsf, stream) = converter.next(&voice);
                    if let Some(lsf) = lsf {
                        let kiss = KissFrame::new_stream_setup(&lsf.0).unwrap();
                        let _ = kiss_out_tx.send(kiss.as_bytes().into());
                    }
                    let kiss = KissFrame::new_stream_data(&stream).unwrap();
                    let _ = kiss_out_tx.send(kiss.as_bytes().into());
                }
                ServerMessage::Ping(_ping) => {
                    let mut pong = Pong::new();
                    pong.set_address(
                        M17Address::from_callsign("VK7XT")
                            .unwrap()
                            .address()
                            .clone(),
                    );
                    if socket.send_to(pong.as_bytes(), dest).is_err() {
                        break;
                    }
                }
                _ => {}
            },
            TncEvent::TransmitVoice(voice) => {
                if socket.send_to(voice.as_bytes(), dest).is_err() {
                    break;
                };
            }
        }
    }
    single_conn_ended.store(true, Ordering::Release);
    status
        .lock()
        .unwrap()
        .status_changed(TncStatus::Disconnected);
}

fn spawn_reader(socket: UdpSocket, event_tx: Sender<TncEvent>, cancel: Arc<AtomicBool>) {
    std::thread::spawn(move || {
        let mut buf = [0u8; 2048];
        while let Ok((n, _sa)) = socket.recv_from(&mut buf) {
            if cancel.load(Ordering::Acquire) {
                break;
            }
            if let Some(msg) = ServerMessage::parse(&buf[..n]) {
                if event_tx.send(TncEvent::Received(msg)).is_err() {
                    break;
                }
            }
        }
    });
}

/// Callbacks to get runtime information about how the reflector client TNC is operating
pub trait StatusHandler {
    fn status_changed(&mut self, status: TncStatus);
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum TncStatus {
    Disconnected,
    Connecting,
    Connected,
    ConnectRejected,
    ForceDisconnect,
    Closed,
}

pub struct NullStatusHandler;
impl StatusHandler for NullStatusHandler {
    fn status_changed(&mut self, _status: TncStatus) {}
}
