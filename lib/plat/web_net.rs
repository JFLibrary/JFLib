// web_net -- the browser's WebSocket (G7d): plat::net::WebSocket through the JS host
//
//   WebSocket::connect(url) / connect_with(url, WsLimits) -> NetResult<WebSocket>
//            "ws://host:port/path", "wss://...", or "/path": the page's own server (ws:// for a
//            page over http, wss:// over https); net.connect is checked on the URL's host and
//            port first (policy::net_host_allowed), a refusal is NetErrorKind::Denied.  The
//            page's own server (the host and port of the page's origin) needs no rule: it served
//            the app (the user's decision of 2026-11-18)
//   ws.state() -> WsState (Connecting, Open, Closing, Closed)
//   ws.send_text(&str) / send_binary(&[u8])     while Open
//   ws.recv() -> Option<WsMessage>                never blocks
//   ws.close(code, reason) / close_info() / protocol()
//
// The worker (web/worker.js) holds the browser's WebSocket objects, so frames never pass the
// page: REQUEST_WEBSOCKET with the socket's id as the request's id, one op byte and its data; the
// host answers through jfgui_ws(id, kind, len) with the bytes in the inbox and wakes the app
// (plat::wake: E_WAKE, the UI runs a pass).  The browser does the handshake, the masking,
// fragments and pings itself.  What a socket keeps for the app is bounded (WsLimits): a bigger
// message or more queued bytes close it with 1009.  The sockets belong to the app's thread (the
// worker that runs it): compute threads of the threaded build do not reach them.

use alloc::collections::VecDeque;
use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, Ordering};

use rt::sync::SpinLock;

use super::{NetError, NetErrorKind, NetResult};

/// (browser) a WebSocket operation for the worker: the socket's id as the request's id, one op
/// byte (1 open: the URL; 2 send text; 3 send binary; 4 close: u16 code, the reason), its data
pub const REQUEST_WEBSOCKET: u32 = 32;
const OP_OPEN: u8 = 1;
const OP_TEXT: u8 = 2;
const OP_BINARY: u8 = 3;
const OP_CLOSE: u8 = 4;
// what jfgui_ws brings
const EV_OPEN: u32 = 1;
const EV_TEXT: u32 = 2;
const EV_BINARY: u32 = 3;
const EV_CLOSED: u32 = 4;
/// sockets an app may hold at once in the browser
pub const WS_MAX_SOCKETS: usize = 64;

/// where a browser WebSocket stands
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WsState {
    /// the browser connects (handshake on the way)
    Connecting,
    /// connected: messages go both ways
    Open,
    /// close() was called, or a limit closed it; the browser's close has not come yet
    Closing,
    /// ended: close_info() says how
    Closed,
}

/// a whole message from the server
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WsMessage {
    /// a text message
    Text(String),
    /// a binary message
    Binary(Vec<u8>),
}

/// what a browser WebSocket keeps for the app until recv takes it
#[derive(Clone, Copy, Debug)]
pub struct WsLimits {
    /// the largest message; a bigger one closes the socket (1009)
    pub max_message: usize,
    /// bytes received and not taken yet; more close the socket (1009)
    pub max_queued: usize,
}

impl Default for WsLimits {
    fn default() -> Self {
        WsLimits { max_message: 1 << 20, max_queued: 8 << 20 }
    }
}

struct Slot {
    id: u32,
    state: WsState,
    inbox: VecDeque<WsMessage>,
    queued: usize,
    lim: WsLimits,
    close: Option<(u16, String)>,
    protocol: String,
}

static SOCKETS: SpinLock<Vec<Slot>> = SpinLock::new(Vec::new());
/// the page's origin as the worker reported it at the start (jfgui_PAGE_ORIGIN): the first value counts,
/// so an app's own setenv cannot widen what "the page's own server" means
static PAGE_ORIGIN: SpinLock<Option<String>> = SpinLock::new(None);
static NEXT_ID: AtomicU32 = AtomicU32::new(1);

/// a WebSocket of the page (the browser's object lives in the worker).  Dropping it closes it
pub struct WebSocket {
    id: u32,
}

fn other() -> NetError {
    NetError::new(NetErrorKind::Other, 0)
}

/// a WebSocket URL -> (the absolute URL, the host as written, the port); "/path" against the
/// page's origin (jfgui_PAGE_ORIGIN, which the worker sets); None: not a ws:// / wss:// URL
pub fn ws_url_split(url: &str, origin: Option<&str>) -> Option<(String, String, u16)> {
    if url.bytes().any(|b| b <= b' ' || b == 0x7f || b == b'\\') || url.contains('#') {
        return None;
    }
    let abs = if url.starts_with('/') && !url.starts_with("//") {
        let o = origin?;
        let (scheme, rest) = o.split_once("://")?;
        let ws = if scheme.eq_ignore_ascii_case("https") {
            "wss"
        } else if scheme.eq_ignore_ascii_case("http") {
            "ws"
        } else {
            return None;
        };
        alloc::format!("{}://{}{}", ws, rest.trim_end_matches('/'), url)
    } else {
        String::from(url)
    };
    let (scheme, rest) = abs.split_once("://")?;
    let default = if scheme.eq_ignore_ascii_case("ws") {
        80
    } else if scheme.eq_ignore_ascii_case("wss") {
        443
    } else {
        return None;
    };
    let end = rest.find(['/', '?']).unwrap_or(rest.len());
    let auth = &rest[..end];
    if auth.is_empty() || auth.contains('@') {
        return None;
    }
    // host: [IPv6], an IPv4 address or a name; then ":port" or nothing
    let (host, port) = if let Some(r) = auth.strip_prefix('[') {
        let (h, p) = r.split_once(']')?;
        super::IpAddr::parse(h)?;
        (h, p)
    } else {
        match auth.find(':') {
            Some(i) => (&auth[..i], &auth[i..]),
            None => (auth, ""),
        }
    };
    let port = match port.strip_prefix(':') {
        Some(n) if !n.is_empty() && n.len() <= 5 && n.bytes().all(|b| b.is_ascii_digit()) => n.parse::<u16>().ok()?,
        None if port.is_empty() => default,
        _ => return None,
    };
    if host.is_empty() {
        return None;
    }
    let host = host.to_ascii_lowercase();
    Some((abs, host, port))
}

/// (web.rs, jfgui_env) the worker's jfgui_PAGE_ORIGIN, kept from its first report on
pub(crate) fn set_page_origin(origin: &str) {
    let mut g = PAGE_ORIGIN.lock();
    if g.is_none() {
        *g = Some(String::from(origin));
    }
}

/// the page's origin (jfgui_PAGE_ORIGIN at the start); None outside a page
pub fn page_origin() -> Option<String> {
    PAGE_ORIGIN.lock().clone()
}

/// whether host and port are the page's own server (the host and port of `origin`, its default port
/// when it names none)
pub fn is_page_server(origin: Option<&str>, host: &str, port: u16) -> bool {
    origin.and_then(|o| ws_url_split("/", Some(o))).is_some_and(|(_, h, p)| h == host && p == port)
}

impl WebSocket {
    /// connect to a WebSocket server with the default limits (see the top of this file)
    pub fn connect(url: &str) -> NetResult<WebSocket> {
        WebSocket::connect_with(url, WsLimits::default())
    }

    /// connect with these limits: Connecting at once, Open when the browser's handshake is done
    pub fn connect_with(url: &str, lim: WsLimits) -> NetResult<WebSocket> {
        let origin = page_origin();
        let (abs, host, port) = ws_url_split(url, origin.as_deref()).ok_or_else(other)?;
        // the page's own server needs no rule (it served the app); every other host its net.connect rule
        if !is_page_server(origin.as_deref(), &host, port) && !crate::policy::net_host_allowed(&host, port) {
            return Err(NetError::new(NetErrorKind::Denied, 0));
        }
        let id = {
            let mut g = SOCKETS.lock();
            if g.len() >= WS_MAX_SOCKETS {
                return Err(other());
            }
            let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
            g.push(Slot { id, state: WsState::Connecting, inbox: VecDeque::new(), queued: 0, lim, close: None, protocol: String::new() });
            id
        };
        let mut p = Vec::with_capacity(abs.len() + 1);
        p.push(OP_OPEN);
        p.extend_from_slice(abs.as_bytes());
        crate::web::notice_id(REQUEST_WEBSOCKET, id, &p);
        Ok(WebSocket { id })
    }

    fn with<R>(&self, f: impl FnOnce(&mut Slot) -> R) -> Option<R> {
        SOCKETS.lock().iter_mut().find(|s| s.id == self.id).map(f)
    }

    /// where it stands
    pub fn state(&self) -> WsState {
        self.with(|s| s.state).unwrap_or(WsState::Closed)
    }

    /// the next message, or None (never waits)
    pub fn recv(&self) -> Option<WsMessage> {
        self.with(|s| {
            let m = s.inbox.pop_front()?;
            s.queued -= match &m {
                WsMessage::Text(t) => t.len(),
                WsMessage::Binary(b) => b.len(),
            };
            Some(m)
        })
        .flatten()
    }

    /// messages waiting for recv
    pub fn pending(&self) -> usize {
        self.with(|s| s.inbox.len()).unwrap_or(0)
    }

    fn send(&self, op: u8, data: &[u8]) -> NetResult<()> {
        match self.state() {
            WsState::Open => {}
            WsState::Connecting => return Err(other()),
            _ => return Err(NetError::new(NetErrorKind::Eof, 0)),
        }
        let mut p = Vec::with_capacity(data.len() + 1);
        p.push(op);
        p.extend_from_slice(data);
        crate::web::notice_id(REQUEST_WEBSOCKET, self.id, &p);
        Ok(())
    }

    /// send a text message (Open only: Other while Connecting, Eof after the close)
    pub fn send_text(&self, s: &str) -> NetResult<()> {
        self.send(OP_TEXT, s.as_bytes())
    }

    /// send a binary message (as send_text)
    pub fn send_binary(&self, b: &[u8]) -> NetResult<()> {
        self.send(OP_BINARY, b)
    }

    /// close with `code` (1000 or 3000-4999, others become 1000; the browser allows no more) and
    /// a reason of at most 123 bytes
    pub fn close(&self, code: u16, reason: &str) {
        let code = if code == 1000 || (3000..=4999).contains(&code) { code } else { 1000 };
        let go = self
            .with(|s| {
                let open = matches!(s.state, WsState::Connecting | WsState::Open);
                if open {
                    s.state = WsState::Closing;
                }
                open
            })
            .unwrap_or(false);
        if go {
            close_host(self.id, code, reason);
        }
    }

    /// how it ended: the close code (1006: no close frame, the browser's word for a failed
    /// connection too) and reason; None while it is not closed
    pub fn close_info(&self) -> Option<(u16, String)> {
        self.with(|s| if s.state == WsState::Closed { s.close.clone() } else { None }).flatten()
    }

    /// the subprotocol the server chose (empty: none)
    pub fn protocol(&self) -> String {
        self.with(|s| s.protocol.clone()).unwrap_or_default()
    }
}

fn close_host(id: u32, code: u16, reason: &str) {
    let mut r = reason;
    while r.len() > 123 {
        let mut k = 123;
        while !r.is_char_boundary(k) {
            k -= 1;
        }
        r = &r[..k];
    }
    let mut p = Vec::with_capacity(r.len() + 3);
    p.push(OP_CLOSE);
    p.extend_from_slice(&code.to_le_bytes());
    p.extend_from_slice(r.as_bytes());
    crate::web::notice_id(REQUEST_WEBSOCKET, id, &p);
}

impl Drop for WebSocket {
    fn drop(&mut self) {
        self.close(1000, "");
        SOCKETS.lock().retain(|s| s.id != self.id);
    }
}

/// (browser host) a WebSocket's event: kind 1 open (the inbox holds the chosen subprotocol),
/// 2 a text message, 3 a binary message, 4 closed (u16 code little-endian, the reason); the host
/// calls jfgui_input after a burst.  -> 1 taken
#[unsafe(no_mangle)]
pub extern "C" fn jfgui_ws(id: u32, kind: u32, len: u32) -> u32 {
    let data = crate::web::input(len).unwrap_or_default();
    let mut close_now = false;
    {
        let mut g = SOCKETS.lock();
        let Some(s) = g.iter_mut().find(|s| s.id == id) else { return 0 };
        match kind {
            EV_OPEN => {
                if s.state == WsState::Connecting {
                    s.state = WsState::Open;
                }
                s.protocol = String::from_utf8(data).unwrap_or_default();
            }
            EV_TEXT | EV_BINARY => {
                if s.state != WsState::Open {
                    return 0; // after our close: dropped
                }
                let n = data.len();
                if n > s.lim.max_message || s.queued.saturating_add(n) > s.lim.max_queued {
                    s.state = WsState::Closing;
                    s.close = Some((1009, String::from("message or queue over the limits")));
                    close_now = true;
                } else {
                    s.queued += n;
                    s.inbox.push_back(if kind == EV_TEXT {
                        WsMessage::Text(String::from_utf8(data).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned()))
                    } else {
                        WsMessage::Binary(data)
                    });
                }
            }
            EV_CLOSED => {
                s.state = WsState::Closed;
                if s.close.is_none() {
                    let code = if data.len() >= 2 { u16::from_le_bytes([data[0], data[1]]) } else { 1006 };
                    let reason = data.get(2..).map(|r| String::from_utf8_lossy(r).into_owned()).unwrap_or_default();
                    s.close = Some((code, reason));
                }
            }
            _ => return 0,
        }
    }
    if close_now {
        // (a page may close with 1000 or 3000-4999 only; the app sees 1009 in close_info)
        close_host(id, 1000, "message over the app's limits");
    }
    crate::wake();
    1
}
