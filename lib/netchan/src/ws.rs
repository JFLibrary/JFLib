// ws -- one WebSocket client type for every target, which never blocks (G7d)
//
//   WebSocket::connect(url) / connect_with(url, WsOptions) -> Result<WebSocket, Error>
//            ws://host:port/path, wss://... (the browser; on the desktop wss:// is part 2), and in
//            the browser "/path" for the page's own server
//   ws.state() -> WsState   send_text / send_binary (queued)   recv() -> Option<WsMessage>
//   ws.close(code, reason) / close_info()
//
// The browser: plat::net::WebSocket (the page's, through the JS host).  Windows, Linux, Apple:
// netio::ws::connect on a thread of its own, which then reads; a second thread writes what send
// queues, so no call waits for the network.  Both wake the UI (plat::wake) when a message comes.
// Android: Unsupported for now.  net.connect is checked on the URL's host and port before
// anything else (Error::Net with NetErrorKind::Denied).

use alloc::string::String;
use alloc::vec::Vec;

use crate::Error;

/// where a WebSocket stands
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WsState {
    /// connecting (TCP, TLS, the upgrade)
    Connecting,
    Open,
    /// close() was called; the end has not come yet
    Closing,
    /// ended: close_info() says how
    Closed,
}

/// a whole message
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WsMessage {
    Text(String),
    Binary(Vec<u8>),
}

/// a WebSocket's limits
#[derive(Clone, Copy, Debug)]
pub struct WsOptions {
    /// the largest message received; bigger ones close the connection (1009)
    pub max_message: usize,
    /// bytes received and not taken by recv; more close the connection (1009)
    pub max_queued: usize,
    /// (desktop) bytes queued by send and not written yet; send fails with Error::Backlog beyond
    pub max_unsent: usize,
    /// (desktop) the connection and the upgrade within this long
    pub connect_ms: u32,
    /// (desktop) nothing received for this long ends the connection
    pub idle_ms: u32,
    /// (desktop) a ping after this long without anything received (0: none)
    pub ping_ms: u32,
}

impl Default for WsOptions {
    fn default() -> Self {
        WsOptions { max_message: 1 << 20, max_queued: 8 << 20, max_unsent: 4 << 20, connect_ms: 10_000, idle_ms: 60_000, ping_ms: 20_000 }
    }
}

// the URL's host and port as written, for the permission check before anything else
fn host_port(url: &str) -> Option<(String, u16)> {
    let (scheme, rest) = url.split_once("://")?;
    let default = if scheme.eq_ignore_ascii_case("ws") {
        80
    } else if scheme.eq_ignore_ascii_case("wss") {
        443
    } else {
        return None;
    };
    let auth = &rest[..rest.find(['/', '?']).unwrap_or(rest.len())];
    let (host, port) = if let Some(r) = auth.strip_prefix('[') {
        r.split_once(']')?
    } else {
        match auth.find(':') {
            Some(i) => (&auth[..i], &auth[i..]),
            None => (auth, ""),
        }
    };
    let port = match port.strip_prefix(':') {
        Some(p) => p.parse().ok()?,
        None if port.is_empty() => default,
        None => return None,
    };
    (!host.is_empty()).then(|| (String::from(host), port))
}

// ------------------------------------------------------------------ the browser

#[cfg(target_arch = "wasm32")]
mod imp {
    use super::*;
    use plat::net::{WsLimits, WsMessage as PMsg, WsState as PState};

    pub struct Inner(plat::net::WebSocket);

    impl Inner {
        pub fn connect(url: &str, o: WsOptions) -> Result<Inner, Error> {
            plat::net::WebSocket::connect_with(url, WsLimits { max_message: o.max_message, max_queued: o.max_queued }).map(Inner).map_err(|e| {
                if e.kind == plat::net::NetErrorKind::Other { Error::Url(String::from(url)) } else { Error::Net(e) }
            })
        }

        pub fn state(&self) -> WsState {
            match self.0.state() {
                PState::Connecting => WsState::Connecting,
                PState::Open => WsState::Open,
                PState::Closing => WsState::Closing,
                PState::Closed => WsState::Closed,
            }
        }

        pub fn send(&self, text: bool, data: &[u8]) -> Result<(), Error> {
            let r = if text { self.0.send_text(core::str::from_utf8(data).map_err(|_| Error::Size)?) } else { self.0.send_binary(data) };
            r.map_err(|_| Error::NotConnected)
        }

        pub fn recv(&self) -> Option<WsMessage> {
            self.0.recv().map(|m| match m {
                PMsg::Text(t) => WsMessage::Text(t),
                PMsg::Binary(b) => WsMessage::Binary(b),
            })
        }

        pub fn close(&self, code: u16, reason: &str) {
            self.0.close(code, reason)
        }

        pub fn close_info(&self) -> Option<(u16, String)> {
            self.0.close_info()
        }

        pub fn unsent(&self) -> usize {
            0
        }
    }
}

// ------------------------------------------------------------------ Windows, Linux, Apple

#[cfg(any(windows, target_os = "linux", target_vendor = "apple"))]
mod imp {
    use super::*;
    use alloc::collections::VecDeque;
    use alloc::sync::Arc;
    use core::sync::atomic::{AtomicU64, AtomicU8, Ordering};
    use netio::ws::{self as nws, Message, WsError};
    use plat::sync::{Condvar, Mutex};

    const CONNECTING: u8 = 0;
    const OPEN: u8 = 1;
    const CLOSING: u8 = 2;
    const CLOSED: u8 = 3;

    struct Outbox {
        q: VecDeque<(bool, Vec<u8>)>,
        bytes: usize,
        close: Option<(u16, String)>,
        gone: bool,
    }

    struct Shared {
        state: AtomicU8,
        closing_since: AtomicU64,
        inbox: Mutex<(VecDeque<WsMessage>, usize)>,
        out: Mutex<Outbox>,
        cv: Condvar,
        end: Mutex<Option<(u16, String)>>,
        o: WsOptions,
    }

    impl Shared {
        fn finish(&self, code: u16, reason: String) {
            let mut e = self.end.lock();
            if e.is_none() {
                *e = Some((code, reason));
            }
            drop(e);
            self.state.store(CLOSED, Ordering::Release);
            let mut o = self.out.lock();
            o.gone = true;
            drop(o);
            self.cv.notify_all();
            plat::wake();
        }

        fn request_close(&self, code: u16, reason: &str) {
            let mut o = self.out.lock();
            if o.close.is_none() {
                o.close = Some((code, String::from(reason)));
            }
            drop(o);
            let _ = self.state.compare_exchange(OPEN, CLOSING, Ordering::AcqRel, Ordering::Acquire);
            let _ = self.state.compare_exchange(CONNECTING, CLOSING, Ordering::AcqRel, Ordering::Acquire);
            let _ = self.closing_since.compare_exchange(0, plat::time_us().max(1), Ordering::AcqRel, Ordering::Acquire);
            self.cv.notify_all();
        }
    }

    pub struct Inner(Arc<Shared>);

    impl Inner {
        pub fn connect(url: &str, o: WsOptions) -> Result<Inner, Error> {
            if nws::parse_url(url).is_none() {
                return Err(Error::Url(String::from(url)));
            }
            let sh = Arc::new(Shared {
                state: AtomicU8::new(CONNECTING),
                closing_since: AtomicU64::new(0),
                inbox: Mutex::new((VecDeque::new(), 0)),
                out: Mutex::new(Outbox { q: VecDeque::new(), bytes: 0, close: None, gone: false }),
                cv: Condvar::new(),
                end: Mutex::new(None),
                o,
            });
            let s = sh.clone();
            let url = String::from(url);
            if !crate::reap::spawn(256 * 1024, move || reader(s, url)) {
                return Err(Error::Net(plat::net::NetError::new(plat::net::NetErrorKind::Other, 0)));
            }
            Ok(Inner(sh))
        }

        pub fn state(&self) -> WsState {
            match self.0.state.load(Ordering::Acquire) {
                CONNECTING => WsState::Connecting,
                OPEN => WsState::Open,
                CLOSING => WsState::Closing,
                _ => WsState::Closed,
            }
        }

        pub fn send(&self, text: bool, data: &[u8]) -> Result<(), Error> {
            if self.0.state.load(Ordering::Acquire) != OPEN {
                return Err(Error::NotConnected);
            }
            if text && core::str::from_utf8(data).is_err() {
                return Err(Error::Size);
            }
            let mut o = self.0.out.lock();
            if o.bytes + data.len() > self.0.o.max_unsent {
                return Err(Error::Backlog);
            }
            o.bytes += data.len();
            o.q.push_back((text, Vec::from(data)));
            drop(o);
            self.0.cv.notify_all();
            Ok(())
        }

        pub fn recv(&self) -> Option<WsMessage> {
            let mut i = self.0.inbox.lock();
            let m = i.0.pop_front()?;
            i.1 -= match &m {
                WsMessage::Text(t) => t.len(),
                WsMessage::Binary(b) => b.len(),
            };
            Some(m)
        }

        pub fn close(&self, code: u16, reason: &str) {
            self.0.request_close(code, reason);
        }

        pub fn close_info(&self) -> Option<(u16, String)> {
            if self.0.state.load(Ordering::Acquire) == CLOSED { self.0.end.lock().clone() } else { None }
        }

        pub fn unsent(&self) -> usize {
            self.0.out.lock().bytes
        }
    }

    impl Drop for Inner {
        fn drop(&mut self) {
            if self.0.state.load(Ordering::Acquire) != CLOSED {
                self.0.request_close(nws::CLOSE_NORMAL, "");
            }
        }
    }

    fn end_of(e: &WsError) -> (u16, String) {
        match e {
            WsError::Closed(c, r) => (*c, r.clone()),
            WsError::Protocol(c) => (*c, String::from("protocol error")),
            WsError::TimedOut => (nws::CLOSE_GOING_AWAY, String::from("timed out")),
            other => (nws::CLOSE_ABNORMAL, alloc::format!("{}", other)),
        }
    }

    // connect, then read until the end; starts the writer once connected
    fn reader(sh: Arc<Shared>, url: String) {
        let cfg = nws::ClientConfig {
            limits: nws::Limits {
                max_message: sh.o.max_message,
                max_frame: sh.o.max_message,
                idle_ms: sh.o.idle_ms,
                ping_ms: sh.o.ping_ms,
                handshake_ms: sh.o.connect_ms,
                ..nws::Limits::default()
            },
            connect_ms: sh.o.connect_ms,
            ..nws::ClientConfig::default()
        };
        let ws = match nws::connect(&url, &cfg) {
            Ok(ws) => Arc::new(ws),
            Err(e) => {
                let (c, r) = end_of(&e);
                sh.finish(c, r);
                return;
            }
        };
        if sh.state.compare_exchange(CONNECTING, OPEN, Ordering::AcqRel, Ordering::Acquire).is_ok() {
            plat::wake();
        }
        let (s, w) = (sh.clone(), ws.clone());
        if !crate::reap::spawn(128 * 1024, move || writer(s, w)) {
            ws.close(nws::CLOSE_INTERNAL, "");
            sh.finish(nws::CLOSE_INTERNAL, String::from("no writer thread"));
            return;
        }
        loop {
            match ws.recv_timeout(Some(200)) {
                Ok(Some(m)) => {
                    let n = m.bytes().len();
                    let mut i = sh.inbox.lock();
                    if i.1 + n > sh.o.max_queued {
                        drop(i);
                        sh.request_close(nws::CLOSE_TOO_BIG, "queue over the limit");
                        continue;
                    }
                    i.1 += n;
                    i.0.push_back(match m {
                        Message::Text(t) => WsMessage::Text(t),
                        Message::Binary(b) => WsMessage::Binary(b),
                    });
                    drop(i);
                    plat::wake();
                }
                Ok(None) => {
                    // our close went out: the peer's answer within a second, else the end
                    let since = sh.closing_since.load(Ordering::Acquire);
                    if since != 0 && plat::time_us().saturating_sub(since) > 1_000_000 {
                        let (c, r) = sh.out.lock().close.clone().unwrap_or((nws::CLOSE_NORMAL, String::new()));
                        sh.finish(c, r);
                        return;
                    }
                }
                Err(e) => {
                    let (c, r) = end_of(&e);
                    sh.finish(c, r);
                    return;
                }
            }
        }
    }

    // write what send queued; the close when it is asked for
    fn writer(sh: Arc<Shared>, ws: Arc<nws::WebSocket>) {
        loop {
            let job = {
                let mut o = sh.out.lock();
                loop {
                    if let Some(item) = o.q.pop_front() {
                        o.bytes -= item.1.len();
                        break Some(item);
                    }
                    if o.close.is_some() || o.gone {
                        break None;
                    }
                    o = sh.cv.wait_timeout(o, Some(1000)).0;
                }
            };
            match job {
                Some((text, data)) => {
                    let r = if text { ws.send_text(core::str::from_utf8(&data).unwrap_or("")) } else { ws.send_binary(&data) };
                    if r.is_err() {
                        return;
                    }
                }
                None => {
                    let close = sh.out.lock().close.clone();
                    if let Some((c, r)) = close {
                        ws.close(c, &r);
                    }
                    return;
                }
            }
        }
    }
}

// ------------------------------------------------------------------ Android (no sockets yet)

#[cfg(not(any(target_arch = "wasm32", windows, target_os = "linux", target_vendor = "apple")))]
mod imp {
    use super::*;

    pub struct Inner;

    impl Inner {
        pub fn connect(_url: &str, _o: WsOptions) -> Result<Inner, Error> {
            Err(Error::Unsupported)
        }
        pub fn state(&self) -> WsState {
            WsState::Closed
        }
        pub fn send(&self, _text: bool, _data: &[u8]) -> Result<(), Error> {
            Err(Error::Unsupported)
        }
        pub fn recv(&self) -> Option<WsMessage> {
            None
        }
        pub fn close(&self, _code: u16, _reason: &str) {}
        pub fn close_info(&self) -> Option<(u16, String)> {
            None
        }
        pub fn unsent(&self) -> usize {
            0
        }
    }
}

/// a WebSocket client that never blocks (see the top of this file); dropping it closes it
pub struct WebSocket {
    inner: imp::Inner,
}

impl WebSocket {
    /// connect with the default limits
    pub fn connect(url: &str) -> Result<WebSocket, Error> {
        WebSocket::connect_with(url, WsOptions::default())
    }

    /// connect with these limits: Connecting at once, Open once the upgrade is done
    pub fn connect_with(url: &str, o: WsOptions) -> Result<WebSocket, Error> {
        // the permissions first (a relative URL of the browser is checked by plat when resolved)
        if let Some((host, port)) = host_port(url) {
            if !plat::policy::net_host_allowed(&host, port) {
                return Err(Error::Net(plat::net::NetError::new(plat::net::NetErrorKind::Denied, 0)));
            }
        } else if !(cfg!(target_arch = "wasm32") && url.starts_with('/')) {
            return Err(Error::Url(String::from(url)));
        }
        imp::Inner::connect(url, o).map(|inner| WebSocket { inner })
    }

    /// where it stands
    pub fn state(&self) -> WsState {
        self.inner.state()
    }

    /// send a text message (Open only; desktop: queued, Backlog beyond max_unsent)
    pub fn send_text(&self, s: &str) -> Result<(), Error> {
        self.inner.send(true, s.as_bytes())
    }

    /// send a binary message (as send_text)
    pub fn send_binary(&self, b: &[u8]) -> Result<(), Error> {
        self.inner.send(false, b)
    }

    /// the next message, or None (never waits)
    pub fn recv(&self) -> Option<WsMessage> {
        self.inner.recv()
    }

    /// close with `code` (1000, 3000-4999) and a reason (at most 123 bytes)
    pub fn close(&self, code: u16, reason: &str) {
        self.inner.close(code, reason)
    }

    /// how it ended (code and reason; 1006: without a close frame or never connected); None
    /// while it is not Closed
    pub fn close_info(&self) -> Option<(u16, String)> {
        self.inner.close_info()
    }

    /// (desktop) bytes queued by send and not written yet (the browser buffers on its own: 0)
    pub fn unsent(&self) -> usize {
        self.inner.unsent()
    }
}
