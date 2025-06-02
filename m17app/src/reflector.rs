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
    kiss::KissFrame,
    reflector::{
        convert::VoiceToRf,
        packet::{Connect, Pong, ServerMessage},
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
        Ok(buf.len())
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
            println!("single conn ended");
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
    socket.send_to(connect.as_bytes(), dest).unwrap();
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
                println!("writer: close");
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
                    socket.send_to(pong.as_bytes(), dest).unwrap();
                }
                _ => {}
            },
        }
    }
    single_conn_ended.store(true, Ordering::Release);
    status
        .lock()
        .unwrap()
        .status_changed(TncStatus::Disconnected);
    println!("write thread terminating");
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
        println!("read thread terminating");
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
