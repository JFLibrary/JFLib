// client -- a game client's channel to its server (G7d)
//
//   Client::connect(Endpoint, ClientConfig) -> Result<Client, Error>    Connecting at once
//   client.poll() -> Option<ClientEvent>    every frame until None: reads, keep-alives, timeouts
//   client.send(payload) -> Result<u16, Error>    the message's sequence number
//   client.send_reliable(message) -> Result<(), Error>    the reliable channel (ClientEvent::Reliable
//                                           on the other side); poll() sends what is due
//   client.state() / reliable() / max_payload() / stats() / rtt_ms() / disconnect()
//
// Nothing blocks.  UDP (Windows, Linux, Apple): a thread of its own reads the socket, keeps only
// the server's datagrams (a bounded queue) and wakes the UI (plat::wake); sends go out at once.
// WebSocket: netchan::ws (the browser's own, or netio's on threads).  In the browser only
// Endpoint::WebSocket exists; on Android nothing yet (Unsupported).

use alloc::string::String;

use crate::endpoint::{ClientConfig, ClientCore, ClientEvent, State};
use crate::packet::DATA_HEADER;
use crate::ws::{WebSocket, WsMessage, WsOptions, WsState};
use crate::{Error, Stats};

#[cfg(any(windows, target_os = "linux", target_vendor = "apple"))]
use alloc::{collections::VecDeque, sync::Arc, vec::Vec};
#[cfg(any(windows, target_os = "linux", target_vendor = "apple"))]
use core::sync::atomic::{AtomicBool, Ordering};

/// datagrams the UDP thread keeps until poll takes them
#[cfg(any(windows, target_os = "linux", target_vendor = "apple"))]
const UDP_QUEUE: usize = 4096;

/// where a client connects to
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Endpoint {
    /// a server's UDP socket (not in the browser)
    Udp(plat::net::SocketAddr),
    /// a server's WebSocket: ws://host:port/path, wss://..., in the browser also "/path"
    WebSocket(String),
}

#[cfg(any(windows, target_os = "linux", target_vendor = "apple"))]
struct Udp {
    sock: Arc<plat::net::UdpSocket>,
    server: plat::net::SocketAddr,
    inbox: Arc<plat::sync::Mutex<VecDeque<Vec<u8>>>>,
    stop: Arc<AtomicBool>,
}

enum Io {
    #[cfg(any(windows, target_os = "linux", target_vendor = "apple"))]
    Udp(Udp),
    Ws { ws: WebSocket, started: bool, max_unsent: usize },
}

/// a client's channel (see the top of this file)
pub struct Client {
    core: ClientCore,
    io: Io,
}

fn now() -> u64 {
    plat::time_us()
}

#[cfg(any(windows, target_os = "linux", target_vendor = "apple"))]
fn udp_reader(sock: Arc<plat::net::UdpSocket>, server: plat::net::SocketAddr, inbox: Arc<plat::sync::Mutex<VecDeque<Vec<u8>>>>, stop: Arc<AtomicBool>) {
    let mut buf = alloc::vec![0u8; 65536];
    while !stop.load(Ordering::Acquire) {
        match sock.recv_from(&mut buf, 100) {
            Ok((n, from)) if from == server => {
                let mut q = inbox.lock();
                if q.len() < UDP_QUEUE {
                    q.push_back(buf[..n].to_vec());
                }
                drop(q);
                plat::wake();
            }
            Ok(_) => {}
            Err(e) if e.kind == plat::net::NetErrorKind::TimedOut => {}
            // (Windows reports an ICMP port unreachable of an earlier send as a reset)
            Err(_) => plat::sleep_ms(5),
        }
    }
}

impl Client {
    /// connect: the handshake starts at once (UDP) or once the WebSocket is open; poll() reports
    /// Connected or Disconnected(reason)
    pub fn connect(ep: Endpoint, cfg: ClientConfig) -> Result<Client, Error> {
        match ep {
            Endpoint::Udp(addr) => {
                #[cfg(any(windows, target_os = "linux", target_vendor = "apple"))]
                {
                    use plat::net::{IpAddr, SocketAddr, UdpSocket};
                    if !plat::policy::net_connect_allowed(&addr) {
                        return Err(Error::Net(plat::net::NetError::new(plat::net::NetErrorKind::Denied, 0)));
                    }
                    let any = if matches!(addr.ip, IpAddr::V4(_)) { IpAddr::UNSPECIFIED_V4 } else { IpAddr::V6([0; 16]) };
                    let sock = Arc::new(UdpSocket::bind(&SocketAddr::new(any, 0))?);
                    let inbox = Arc::new(plat::sync::Mutex::new(VecDeque::new()));
                    let stop = Arc::new(AtomicBool::new(false));
                    let (s, i, st) = (sock.clone(), inbox.clone(), stop.clone());
                    if !crate::reap::spawn(128 * 1024, move || udp_reader(s, addr, i, st)) {
                        return Err(Error::Net(plat::net::NetError::new(plat::net::NetErrorKind::Other, 0)));
                    }
                    let mut core = ClientCore::new(cfg, plat::random_bytes::<32>(), false);
                    core.start(now());
                    let mut c = Client { core, io: Io::Udp(Udp { sock, server: addr, inbox, stop }) };
                    c.flush();
                    Ok(c)
                }
                #[cfg(not(any(windows, target_os = "linux", target_vendor = "apple")))]
                {
                    let _ = (addr, cfg);
                    Err(Error::Unsupported)
                }
            }
            Endpoint::WebSocket(url) => {
                let o = WsOptions { max_message: cfg.max_payload + DATA_HEADER, connect_ms: cfg.connect_ms, idle_ms: cfg.timeout_ms.saturating_mul(2), ping_ms: 0, ..WsOptions::default() };
                let ws = WebSocket::connect_with(&url, o)?;
                Ok(Client { core: ClientCore::new(cfg, ws_random(), true), io: Io::Ws { ws, started: false, max_unsent: o.max_unsent } })
            }
        }
    }

    // what came, the clock, what is to send
    fn pump(&mut self) {
        let t = now();
        match &mut self.io {
            #[cfg(any(windows, target_os = "linux", target_vendor = "apple"))]
            Io::Udp(u) => {
                let got: Vec<Vec<u8>> = u.inbox.lock().drain(..).collect();
                for d in got {
                    self.core.packet(&d, t);
                }
            }
            Io::Ws { ws, started, .. } => {
                if !*started && ws.state() == WsState::Open {
                    *started = true;
                    self.core.start(t);
                }
                while let Some(m) = ws.recv() {
                    if let WsMessage::Binary(b) = m {
                        self.core.packet(&b, t);
                    }
                }
                if ws.state() == WsState::Closed {
                    let code = ws.close_info().map(|c| c.0).unwrap_or(1006);
                    self.core.transport_closed(code);
                }
            }
        }
        self.core.tick(t);
        self.flush();
    }

    fn flush(&mut self) {
        while let Some(b) = self.core.take_out() {
            match &self.io {
                #[cfg(any(windows, target_os = "linux", target_vendor = "apple"))]
                Io::Udp(u) => {
                    let _ = u.sock.send_to(&b, &u.server);
                }
                Io::Ws { ws, .. } => {
                    let _ = ws.send_binary(&b);
                }
            }
        }
    }

    /// the next event; call it until None every frame (it reads, sends keep-alives, notices
    /// timeouts)
    pub fn poll(&mut self) -> Option<ClientEvent> {
        if let Some(e) = self.core.poll_event() {
            return Some(e);
        }
        self.pump();
        self.core.poll_event()
    }

    /// send a message -> its sequence number (Delivered / Lost name it later; Backlog: the
    /// WebSocket has WsOptions::max_unsent bytes waiting, nothing was sent)
    pub fn send(&mut self, payload: &[u8]) -> Result<u16, Error> {
        let backlog = match &self.io {
            Io::Ws { ws, max_unsent, .. } => ws.unsent() + payload.len() + DATA_HEADER > *max_unsent,
            #[cfg(any(windows, target_os = "linux", target_vendor = "apple"))]
            Io::Udp(_) => false,
        };
        if backlog {
            return Err(Error::Backlog);
        }
        let seq = self.core.send(payload, now())?;
        self.flush();
        Ok(seq)
    }

    /// send a message on the reliable channel: any size up to ClientConfig::max_message, in
    /// order, over UDP sent again until the server has it (endpoint::ClientCore::send_reliable).
    /// It goes out at reliable_bytes_per_s, the rest with the next polls
    pub fn send_reliable(&mut self, payload: &[u8]) -> Result<(), Error> {
        self.core.send_reliable(payload, now())?;
        self.flush();
        Ok(())
    }

    /// where it stands
    pub fn state(&self) -> State {
        self.core.state()
    }

    /// whether the transport is reliable and ordered (a WebSocket)
    pub fn reliable(&self) -> bool {
        self.core.reliable()
    }

    /// the largest payload the server takes (0 before Connected)
    pub fn max_payload(&self) -> usize {
        self.core.max_payload()
    }

    /// the connection's counts
    pub fn stats(&self) -> Stats {
        self.core.stats()
    }

    /// the smoothed round-trip time in ms (0 before the first ack)
    pub fn rtt_ms(&self) -> f32 {
        self.core.stats().rtt_ms
    }

    /// end the connection (DISCONNECT goes out; Disconnected(Local) follows)
    pub fn disconnect(&mut self) {
        self.core.disconnect();
        self.flush();
        match &self.io {
            Io::Ws { ws, .. } => ws.close(1000, ""),
            #[cfg(any(windows, target_os = "linux", target_vendor = "apple"))]
            Io::Udp(_) => {}
        }
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        if self.core.state() == State::Connected {
            self.disconnect();
        }
        #[cfg(any(windows, target_os = "linux", target_vendor = "apple"))]
        if let Io::Udp(u) = &self.io {
            u.stop.store(true, Ordering::Release);
        }
    }
}

// the random bytes of a WebSocket attempt (only its nonce comes from them): random where the
// system has a generator; the browser's needs none that is secret (TCP proved the address, and a
// WebSocket has no keys of its own)
fn ws_random() -> [u8; 32] {
    #[cfg(any(windows, target_os = "linux", target_vendor = "apple", target_os = "android"))]
    {
        plat::random_bytes::<32>()
    }
    #[cfg(not(any(windows, target_os = "linux", target_vendor = "apple", target_os = "android")))]
    {
        let mut r = [0u8; 32];
        r[..8].copy_from_slice(&(plat::time_us() ^ 0x9e37_79b9_7f4a_7c15).to_le_bytes());
        r
    }
}
