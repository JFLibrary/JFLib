// ws -- WebSocket (RFC 6455): the frame codec, the server side in netio's HTTP server, a client (G7d)
//
//   Acceptor::new(Limits) [.origins(..)] [.protocols(..)]   shared by a Server's handler
//   acceptor.accept(&req, |ws| ..) -> Response    101 with the upgrade (on_open runs on a thread
//                                     of its own until it returns), or 400 / 403 / 426 / 503
//   connect(url, &ClientConfig) -> WebSocket       ws://host[:port]/path (wss:// with a connector)
//   WebSocket: recv() / recv_timeout(ms) -> Message; send_text / send_binary / ping / close(code,
//              reason); &self everywhere: one thread reads while others send (Arc<WebSocket>)
//   Decoder: untrusted bytes -> Frame events, no I/O (the fuzzing in apps/nettest drives it)
//   encode_frame(out, opcode, fin, payload, mask)
//
// Limits: a frame's and a message's size, frames per message, messages and bytes a second
// (token buckets with a burst of two seconds), idle and handshake timeouts, connections per
// Acceptor.  A malformed or excessive peer gets the close code RFC 6455 gives it (1002 protocol,
// 1007 bad UTF-8 in text, 1008 rate, 1009 too big) and the connection ends; nothing panics, no
// allocation goes past the limits.  No extensions (permessage-deflate) are offered or accepted.

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering};

use plat::net::{NetError, NetErrorKind, SocketAddr, TcpStream};
use plat::sync::Mutex;

use crate::client::Connector;
use crate::http::{self, Body, Headers, Reader, Request, Response, Upgrade, Upgraded};
use crate::url::Url;
use crate::Stream;

/// the GUID RFC 6455 appends to Sec-WebSocket-Key
const GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

/// a continuation frame
pub const OP_CONT: u8 = 0x0;
/// a text frame (UTF-8)
pub const OP_TEXT: u8 = 0x1;
/// a binary frame
pub const OP_BINARY: u8 = 0x2;
/// a close frame
pub const OP_CLOSE: u8 = 0x8;
/// a ping
pub const OP_PING: u8 = 0x9;
/// a pong
pub const OP_PONG: u8 = 0xA;

/// close code: normal end
pub const CLOSE_NORMAL: u16 = 1000;
/// close code: the endpoint goes away (server stop, idle)
pub const CLOSE_GOING_AWAY: u16 = 1001;
/// close code: a protocol error
pub const CLOSE_PROTOCOL: u16 = 1002;
/// close code: a kind of data the endpoint does not take
pub const CLOSE_UNSUPPORTED: u16 = 1003;
/// (never sent) a close frame without a code
pub const CLOSE_NO_STATUS: u16 = 1005;
/// (never sent) the connection ended without a close frame
pub const CLOSE_ABNORMAL: u16 = 1006;
/// close code: a text message that is not UTF-8
pub const CLOSE_BAD_DATA: u16 = 1007;
/// close code: a policy was violated (rate limits)
pub const CLOSE_POLICY: u16 = 1008;
/// close code: a frame or message over the limits
pub const CLOSE_TOO_BIG: u16 = 1009;
/// close code: the server failed
pub const CLOSE_INTERNAL: u16 = 1011;

/// the most a feed may bring beyond one frame (a read of the stream)
const FEED_SLACK: usize = 64 * 1024;
/// the read slice: stop flags, pings and deadlines are looked at this often
const SLICE_MS: u32 = 250;

// ------------------------------------------------------------------ limits and errors

/// the limits of one connection
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// the largest message (all its frames together)
    pub max_message: usize,
    /// the largest frame
    pub max_frame: usize,
    /// frames per message
    pub max_fragments: usize,
    /// outgoing messages longer than this go as several frames
    pub send_frame: usize,
    /// messages (and control frames) a second from the peer, on average; bursts of two seconds
    pub messages_per_s: u32,
    /// bytes a second from the peer, on average; bursts of two seconds
    pub bytes_per_s: u32,
    /// nothing received for this long ends the connection (1001)
    pub idle_ms: u32,
    /// a ping goes out after this long without anything received (0: none)
    pub ping_ms: u32,
    /// (client) the connection and the server's answer within this long
    pub handshake_ms: u32,
    /// the longest a send waits for the peer to take bytes
    pub write_ms: u32,
    /// (Acceptor) connections at once
    pub max_connections: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_message: 1 << 20,
            max_frame: 1 << 20,
            max_fragments: 4096,
            send_frame: 64 * 1024,
            messages_per_s: 2000,
            bytes_per_s: 16 << 20,
            idle_ms: 60_000,
            ping_ms: 20_000,
            handshake_ms: 10_000,
            write_ms: 10_000,
            max_connections: 1024,
        }
    }
}

/// why a WebSocket call failed
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum WsError {
    /// not a ws:// or wss:// URL, or wss:// without a connector
    Url(String),
    /// the network (connect, read, write)
    Net(NetError),
    /// the TLS layer refused (certificate, handshake)
    Tls(String),
    /// the server's answer was not the upgrade (its status, a missing or wrong header)
    Handshake(String),
    /// the connection ended: the peer's close code and reason (1005: its close frame had no
    /// code, 1006: no close frame came)
    Closed(u16, String),
    /// this side ended it: the peer broke the protocol or a limit (the close code sent)
    Protocol(u16),
    /// a message to send over max_message, or a control payload over 125 bytes
    TooLarge,
    /// nothing came within idle_ms (the connection is closed), or the handshake took too long
    TimedOut,
}

impl core::fmt::Display for WsError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            WsError::Url(s) => write!(f, "invalid address: {}", s),
            WsError::Net(e) => write!(f, "{}", e),
            WsError::Tls(s) => write!(f, "TLS: {}", s),
            WsError::Handshake(s) => write!(f, "no WebSocket upgrade: {}", s),
            WsError::Closed(c, r) if r.is_empty() => write!(f, "closed ({})", c),
            WsError::Closed(c, r) => write!(f, "closed ({}: {})", c, r),
            WsError::Protocol(c) => write!(f, "protocol error, closed with {}", c),
            WsError::TooLarge => f.write_str("message too large"),
            WsError::TimedOut => f.write_str("timed out"),
        }
    }
}

impl From<NetError> for WsError {
    fn from(e: NetError) -> Self {
        WsError::Net(e)
    }
}

/// a whole message
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Message {
    /// a text message (UTF-8, checked)
    Text(String),
    /// a binary message
    Binary(Vec<u8>),
}

impl Message {
    /// the payload's bytes
    pub fn bytes(&self) -> &[u8] {
        match self {
            Message::Text(s) => s.as_bytes(),
            Message::Binary(b) => b,
        }
    }
}

/// whether a close code may appear in a close frame (RFC 6455 7.4: 1000-1003, 1007-1014 and the
/// ranges 3000-4999 for libraries and applications)
pub fn close_code_valid(code: u16) -> bool {
    matches!(code, 1000..=1003 | 1007..=1014 | 3000..=4999)
}

// ------------------------------------------------------------------ the frame codec

/// which side a decoder reads for: a server reads masked frames from clients, a client
/// unmasked frames from servers
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// reads a client's frames: masked
    Server,
    /// reads a server's frames: unmasked
    Client,
}

/// one event of the incoming frames
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Frame {
    /// a whole message (its fragments put together)
    Message(Message),
    /// a ping with its payload (at most 125 bytes)
    Ping(Vec<u8>),
    /// a pong with its payload
    Pong(Vec<u8>),
    /// a close frame: its code and reason (None: it had no code)
    Close(Option<(u16, String)>),
}

/// xor `data` with the mask, starting at byte `offset` of the payload
pub fn apply_mask(data: &mut [u8], mask: [u8; 4], offset: usize) {
    let k = (offset & 3) as u32;
    let m = u32::from_le_bytes(mask).rotate_right(8 * k);
    let m8 = (m as u64) | ((m as u64) << 32);
    let mut chunks = data.chunks_exact_mut(8);
    for c in &mut chunks {
        let v = u64::from_le_bytes([c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]]) ^ m8;
        c.copy_from_slice(&v.to_le_bytes());
    }
    let mb = m.to_le_bytes();
    for (i, b) in chunks.into_remainder().iter_mut().enumerate() {
        *b ^= mb[i & 3];
    }
}

/// append one frame to `out`: header, the mask key (a client's frames), the payload (masked)
pub fn encode_frame(out: &mut Vec<u8>, op: u8, fin: bool, payload: &[u8], mask: Option<[u8; 4]>) {
    out.push(if fin { 0x80 } else { 0 } | (op & 0x0f));
    let m = if mask.is_some() { 0x80 } else { 0 };
    let n = payload.len();
    if n < 126 {
        out.push(m | n as u8);
    } else if n <= 0xffff {
        out.push(m | 126);
        out.extend_from_slice(&(n as u16).to_be_bytes());
    } else {
        out.push(m | 127);
        out.extend_from_slice(&(n as u64).to_be_bytes());
    }
    let start = out.len() + if mask.is_some() { 4 } else { 0 };
    if let Some(k) = mask {
        out.extend_from_slice(&k);
    }
    out.extend_from_slice(payload);
    if let Some(k) = mask {
        apply_mask(&mut out[start..], k, 0);
    }
}

/// the frames of untrusted bytes: feed what arrives, take events until next() says it needs
/// more.  Memory stays below one frame (max_frame) plus one feed (64 KiB) plus one message
/// (max_message); a feed beyond that, or anything malformed, is an error with the close code
pub struct Decoder {
    role: Role,
    lim: Limits,
    buf: Vec<u8>,
    pos: usize,
    // the message in progress: its opcode (0: none), its bytes, its frames
    op: u8,
    msg: Vec<u8>,
    frames: usize,
    done: bool,
}

impl Decoder {
    /// a decoder for frames a `role` reads, within `lim`
    pub fn new(role: Role, lim: Limits) -> Decoder {
        Decoder { role, lim, buf: Vec::new(), pos: 0, op: 0, msg: Vec::new(), frames: 0, done: false }
    }

    /// bytes from the peer; Err(1009) when that would buffer more than one frame and a feed
    /// (the caller fed again before next() asked for more)
    pub fn feed(&mut self, data: &[u8]) -> Result<(), u16> {
        if self.pos > 0 {
            self.buf.drain(..self.pos);
            self.pos = 0;
        }
        if self.buf.len().saturating_add(data.len()) > self.lim.max_frame.saturating_add(14 + FEED_SLACK) {
            return Err(CLOSE_TOO_BIG);
        }
        self.buf.extend_from_slice(data);
        Ok(())
    }

    /// bytes fed but not decoded yet
    pub fn buffered(&self) -> usize {
        self.buf.len() - self.pos
    }

    /// whether a close frame came (nothing is decoded after it)
    pub fn closed(&self) -> bool {
        self.done
    }

    /// the next event: Ok(None) when more bytes are needed, Err(close code) for a malformed or
    /// excessive stream (the decoder takes nothing more then)
    pub fn next(&mut self) -> Result<Option<Frame>, u16> {
        let r = self.step();
        if r.is_err() {
            self.done = true;
            self.buf = Vec::new();
            self.msg = Vec::new();
            self.pos = 0;
        }
        r
    }

    fn step(&mut self) -> Result<Option<Frame>, u16> {
        loop {
            if self.done {
                return Ok(None);
            }
            let b = &self.buf[self.pos..];
            if b.len() < 2 {
                return Ok(None);
            }
            let (b0, b1) = (b[0], b[1]);
            let fin = b0 & 0x80 != 0;
            let op = b0 & 0x0f;
            if b0 & 0x70 != 0 {
                return Err(CLOSE_PROTOCOL); // reserved bits without an extension
            }
            if !matches!(op, OP_CONT | OP_TEXT | OP_BINARY | OP_CLOSE | OP_PING | OP_PONG) {
                return Err(CLOSE_PROTOCOL);
            }
            let control = op >= 8;
            let masked = b1 & 0x80 != 0;
            let len7 = (b1 & 0x7f) as usize;
            if control && (!fin || len7 > 125) {
                return Err(CLOSE_PROTOCOL);
            }
            if masked != (self.role == Role::Server) {
                return Err(CLOSE_PROTOCOL); // clients mask every frame, servers none
            }
            let ext = match len7 {
                126 => 2,
                127 => 8,
                _ => 0,
            };
            let hlen = 2 + ext + if masked { 4 } else { 0 };
            if b.len() < hlen {
                return Ok(None);
            }
            let len: u64 = match ext {
                2 => {
                    let n = u16::from_be_bytes([b[2], b[3]]) as u64;
                    if n < 126 {
                        return Err(CLOSE_PROTOCOL); // not the shortest form
                    }
                    n
                }
                8 => {
                    let n = u64::from_be_bytes([b[2], b[3], b[4], b[5], b[6], b[7], b[8], b[9]]);
                    if n >> 63 != 0 || n <= 0xffff {
                        return Err(CLOSE_PROTOCOL);
                    }
                    n
                }
                _ => len7 as u64,
            };
            if len > self.lim.max_frame as u64 {
                return Err(CLOSE_TOO_BIG);
            }
            let len = len as usize;
            if !control && (self.msg.len() + len > self.lim.max_message) {
                return Err(CLOSE_TOO_BIG);
            }
            if b.len() - hlen < len {
                return Ok(None);
            }
            let mask = if masked { Some([b[hlen - 4], b[hlen - 3], b[hlen - 2], b[hlen - 1]]) } else { None };
            let start = self.pos + hlen;
            self.pos = start + len;
            let mut payload = self.buf[start..start + len].to_vec();
            if let Some(k) = mask {
                apply_mask(&mut payload, k, 0);
            }
            match op {
                OP_PING => return Ok(Some(Frame::Ping(payload))),
                OP_PONG => return Ok(Some(Frame::Pong(payload))),
                OP_CLOSE => {
                    self.done = true;
                    if payload.is_empty() {
                        return Ok(Some(Frame::Close(None)));
                    }
                    if payload.len() < 2 {
                        return Err(CLOSE_PROTOCOL);
                    }
                    let code = u16::from_be_bytes([payload[0], payload[1]]);
                    if !close_code_valid(code) {
                        return Err(CLOSE_PROTOCOL);
                    }
                    let reason = String::from_utf8(payload[2..].to_vec()).map_err(|_| CLOSE_BAD_DATA)?;
                    return Ok(Some(Frame::Close(Some((code, reason)))));
                }
                OP_CONT => {
                    if self.op == 0 {
                        return Err(CLOSE_PROTOCOL); // a continuation of nothing
                    }
                    self.frames += 1;
                    if self.frames > self.lim.max_fragments {
                        return Err(CLOSE_POLICY);
                    }
                    self.msg.extend_from_slice(&payload);
                }
                _ => {
                    if self.op != 0 {
                        return Err(CLOSE_PROTOCOL); // a new message inside a fragmented one
                    }
                    self.op = op;
                    self.frames = 1;
                    if fin {
                        self.op = 0;
                        return Self::message(op, payload).map(Some);
                    }
                    self.msg = payload;
                }
            }
            if fin {
                let op = core::mem::replace(&mut self.op, 0);
                let msg = core::mem::take(&mut self.msg);
                return Self::message(op, msg).map(Some);
            }
        }
    }

    fn message(op: u8, bytes: Vec<u8>) -> Result<Frame, u16> {
        if op == OP_TEXT {
            String::from_utf8(bytes).map(|s| Frame::Message(Message::Text(s))).map_err(|_| CLOSE_BAD_DATA)
        } else {
            Ok(Frame::Message(Message::Binary(bytes)))
        }
    }
}

// ------------------------------------------------------------------ rate buckets

/// a token bucket: `rate` a second, at most two seconds' worth saved up
#[derive(Clone, Copy)]
struct Bucket {
    rate: u64,
    // tokens in millionths
    level: u64,
    last_us: u64,
}

impl Bucket {
    fn new(rate: u32, now: u64) -> Bucket {
        let rate = rate.max(1) as u64;
        Bucket { rate, level: rate * 2 * 1_000_000, last_us: now }
    }

    fn take(&mut self, n: u64, now: u64) -> bool {
        let cap = self.rate * 2 * 1_000_000;
        let dt = now.saturating_sub(self.last_us);
        self.last_us = now;
        self.level = self.level.saturating_add(dt.saturating_mul(self.rate)).min(cap);
        let need = n.saturating_mul(1_000_000);
        if self.level < need {
            return false;
        }
        self.level -= need;
        true
    }
}

// ------------------------------------------------------------------ the connection

const OPEN: u8 = 0;
const CLOSE_SENT: u8 = 1;
const CLOSED: u8 = 2;

struct ReadSide {
    dec: Decoder,
    scratch: Vec<u8>,
    msgs: Bucket,
    bytes: Bucket,
    last_rx_us: u64,
    ping_out: bool,
    // how it ended (the error recv returns from then on)
    end: Option<WsError>,
}

struct WriteSide {
    buf: Vec<u8>,
    // client masks: random bytes taken four at a time, refilled from the system's generator
    rand: [u8; 256],
    rpos: usize,
}

impl WriteSide {
    fn mask(&mut self) -> [u8; 4] {
        if self.rpos + 4 > self.rand.len() {
            plat::fill_random(&mut self.rand);
            self.rpos = 0;
        }
        let k = [self.rand[self.rpos], self.rand[self.rpos + 1], self.rand[self.rpos + 2], self.rand[self.rpos + 3]];
        self.rpos += 4;
        k
    }
}

/// one end of a WebSocket connection.  `&self` everywhere: one thread may block in recv while
/// others send (`Arc<WebSocket>`).  Dropping it closes the connection (1000) if it is open
pub struct WebSocket {
    s: Box<dyn Stream>,
    role: Role,
    lim: Limits,
    rd: Mutex<ReadSide>,
    wr: Mutex<WriteSide>,
    state: AtomicU8,
    stop: Option<Arc<AtomicBool>>,
    peer: Option<SocketAddr>,
    protocol: Option<String>,
    _slot: Option<Slot>,
}

/// an Acceptor's count of its open connections, one down when the connection goes
struct Slot(Arc<AtomicUsize>);

impl Drop for Slot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

impl WebSocket {
    fn new(s: Box<dyn Stream>, role: Role, lim: Limits, buffered: &[u8], stop: Option<Arc<AtomicBool>>, peer: Option<SocketAddr>, protocol: Option<String>, slot: Option<Slot>) -> Result<WebSocket, WsError> {
        let now = plat::time_us();
        let mut dec = Decoder::new(role, lim);
        dec.feed(buffered).map_err(WsError::Protocol)?;
        s.set_timeouts(Some(lim.write_ms.max(SLICE_MS)), Some(lim.write_ms));
        let rd = ReadSide {
            dec,
            scratch: alloc::vec![0u8; 16 * 1024],
            msgs: Bucket::new(lim.messages_per_s, now),
            bytes: Bucket::new(lim.bytes_per_s, now),
            last_rx_us: now,
            ping_out: false,
            end: None,
        };
        let wr = WriteSide { buf: Vec::new(), rand: [0u8; 256], rpos: 256 };
        Ok(WebSocket { s, role, lim, rd: Mutex::new(rd), wr: Mutex::new(wr), state: AtomicU8::new(OPEN), stop, peer, protocol, _slot: slot })
    }

    /// the peer's address (server side)
    pub fn peer(&self) -> Option<SocketAddr> {
        self.peer
    }

    /// the subprotocol both sides agreed on (Sec-WebSocket-Protocol)
    pub fn protocol(&self) -> Option<&str> {
        self.protocol.as_deref()
    }

    /// the limits of this connection
    pub fn limits(&self) -> &Limits {
        &self.lim
    }

    /// whether the connection is open (no close sent or received)
    pub fn is_open(&self) -> bool {
        self.state.load(Ordering::Acquire) == OPEN
    }

    /// the next message, waiting as long as it takes (pings are answered, the peer's close is
    /// returned as Err(Closed))
    pub fn recv(&self) -> Result<Message, WsError> {
        loop {
            if let Some(m) = self.recv_timeout(Some(60_000))? {
                return Ok(m);
            }
        }
    }

    /// the next message within `ms` (None: wait as long as it takes) -> Ok(None) when none came
    pub fn recv_timeout(&self, ms: Option<u32>) -> Result<Option<Message>, WsError> {
        let deadline = ms.map(|m| plat::time_us() + m as u64 * 1000);
        let mut rd = self.rd.lock();
        loop {
            if let Some(e) = &rd.end {
                return Err(e.clone());
            }
            let now = plat::time_us();
            let next = rd.dec.next();
            match next {
                Err(code) => return Err(self.fail(&mut rd, code)),
                Ok(Some(frame)) => {
                    if !rd.msgs.take(1, now) {
                        return Err(self.fail(&mut rd, CLOSE_POLICY));
                    }
                    match frame {
                        Frame::Ping(p) => {
                            let _ = self.send_frame(OP_PONG, &p);
                        }
                        Frame::Pong(_) => rd.ping_out = false,
                        Frame::Close(c) => {
                            let (code, reason) = c.unwrap_or((CLOSE_NO_STATUS, String::new()));
                            if self.state.swap(CLOSED, Ordering::AcqRel) == OPEN {
                                // answer with the same code (none for a close without one)
                                let mut p = Vec::new();
                                if code != CLOSE_NO_STATUS {
                                    p.extend_from_slice(&code.to_be_bytes());
                                }
                                let _ = self.write_frame(OP_CLOSE, &p);
                            }
                            self.s.shutdown();
                            let e = WsError::Closed(code, reason);
                            rd.end = Some(e.clone());
                            return Err(e);
                        }
                        Frame::Message(m) => return Ok(Some(m)),
                    }
                    continue;
                }
                Ok(None) => {}
            }
            if deadline.is_some_and(|d| now >= d) {
                return Ok(None);
            }
            let quiet_ms = now.saturating_sub(rd.last_rx_us) / 1000;
            if quiet_ms >= self.lim.idle_ms as u64 {
                self.fail(&mut rd, CLOSE_GOING_AWAY);
                rd.end = Some(WsError::TimedOut);
                return Err(WsError::TimedOut);
            }
            if self.lim.ping_ms > 0 && quiet_ms >= self.lim.ping_ms as u64 && !rd.ping_out {
                rd.ping_out = true;
                let _ = self.send_frame(OP_PING, b"jf");
            }
            if self.stop.as_ref().is_some_and(|s| s.load(Ordering::Relaxed)) {
                self.fail(&mut rd, CLOSE_GOING_AWAY);
                let e = WsError::Closed(CLOSE_GOING_AWAY, String::from("server stops"));
                rd.end = Some(e.clone());
                return Err(e);
            }
            let mut wait = SLICE_MS as u64;
            if let Some(d) = deadline {
                wait = wait.min(d.saturating_sub(now).div_ceil(1000));
            }
            wait = wait.min((self.lim.idle_ms as u64).saturating_sub(quiet_ms).max(1));
            if !self.s.wait_readable(wait.max(1) as u32) {
                continue;
            }
            let ReadSide { scratch, .. } = &mut *rd;
            let got = self.s.read(scratch);
            match got {
                Ok(0) => {
                    self.state.store(CLOSED, Ordering::Release);
                    let e = WsError::Closed(CLOSE_ABNORMAL, String::new());
                    rd.end = Some(e.clone());
                    return Err(e);
                }
                Ok(n) => {
                    let now = plat::time_us();
                    rd.last_rx_us = now;
                    rd.ping_out = false;
                    if !rd.bytes.take(n as u64, now) {
                        return Err(self.fail(&mut rd, CLOSE_POLICY));
                    }
                    let ReadSide { scratch, dec, .. } = &mut *rd;
                    if let Err(code) = dec.feed(&scratch[..n]) {
                        return Err(self.fail(&mut rd, code));
                    }
                }
                Err(e) if e.kind == NetErrorKind::TimedOut => {}
                Err(e) => {
                    self.state.store(CLOSED, Ordering::Release);
                    rd.end = Some(WsError::Net(e));
                    return Err(WsError::Net(e));
                }
            }
        }
    }

    // the peer broke the protocol or a limit: our close with `code`, the end
    fn fail(&self, rd: &mut ReadSide, code: u16) -> WsError {
        if self.state.swap(CLOSED, Ordering::AcqRel) == OPEN {
            let _ = self.write_frame(OP_CLOSE, &code.to_be_bytes());
        }
        self.s.shutdown();
        let e = WsError::Protocol(code);
        rd.end = Some(e.clone());
        e
    }

    /// send a text message
    pub fn send_text(&self, s: &str) -> Result<(), WsError> {
        self.send_frame(OP_TEXT, s.as_bytes())
    }

    /// send a binary message
    pub fn send_binary(&self, b: &[u8]) -> Result<(), WsError> {
        self.send_frame(OP_BINARY, b)
    }

    /// send a ping (at most 125 bytes); the peer's pong is taken by recv
    pub fn ping(&self, payload: &[u8]) -> Result<(), WsError> {
        self.send_frame(OP_PING, payload)
    }

    /// send a message as frames of exactly `frame` bytes (tests of the peer's reassembly)
    pub fn send_fragmented(&self, op: u8, payload: &[u8], frame: usize) -> Result<(), WsError> {
        if op != OP_TEXT && op != OP_BINARY {
            return Err(WsError::Protocol(CLOSE_PROTOCOL));
        }
        self.send_frames(op, payload, frame.max(1))
    }

    fn send_frame(&self, op: u8, payload: &[u8]) -> Result<(), WsError> {
        if op >= 8 {
            if payload.len() > 125 {
                return Err(WsError::TooLarge);
            }
            return self.write_frame(op, payload);
        }
        self.send_frames(op, payload, self.lim.send_frame.max(1))
    }

    fn send_frames(&self, op: u8, payload: &[u8], frame: usize) -> Result<(), WsError> {
        if payload.len() > self.lim.max_message {
            return Err(WsError::TooLarge);
        }
        let mut wr = self.wr.lock();
        if self.state.load(Ordering::Acquire) != OPEN {
            return Err(self.ended());
        }
        let mut buf = core::mem::take(&mut wr.buf);
        buf.clear();
        let mut chunks = payload.chunks(frame).peekable();
        let mut first = true;
        if payload.is_empty() {
            let m = if self.role == Role::Client { Some(wr.mask()) } else { None };
            encode_frame(&mut buf, op, true, &[], m);
        }
        while let Some(c) = chunks.next() {
            let m = if self.role == Role::Client { Some(wr.mask()) } else { None };
            encode_frame(&mut buf, if first { op } else { OP_CONT }, chunks.peek().is_none(), c, m);
            first = false;
        }
        let r = self.s.write_all(&buf);
        if buf.capacity() <= 256 * 1024 {
            wr.buf = buf;
        }
        r.map_err(|e| {
            self.state.store(CLOSED, Ordering::Release);
            WsError::Net(e)
        })
    }

    // one frame now (control frames, close): also while closing
    fn write_frame(&self, op: u8, payload: &[u8]) -> Result<(), WsError> {
        let mut wr = self.wr.lock();
        if op != OP_CLOSE && self.state.load(Ordering::Acquire) == CLOSED {
            return Err(self.ended());
        }
        let m = if self.role == Role::Client { Some(wr.mask()) } else { None };
        let mut buf = Vec::with_capacity(payload.len() + 14);
        encode_frame(&mut buf, op, true, payload, m);
        self.s.write_all(&buf).map_err(WsError::Net)
    }

    fn ended(&self) -> WsError {
        match self.rd.try_lock().and_then(|r| r.end.clone()) {
            Some(e) => e,
            None => WsError::Closed(CLOSE_NORMAL, String::new()),
        }
    }

    /// close the connection: our close frame with `code` (1000, 3000-4999 ...) and a reason of at
    /// most 123 bytes, then (when no other thread reads) wait up to a second for the peer's close
    pub fn close(&self, code: u16, reason: &str) {
        if self.state.compare_exchange(OPEN, CLOSE_SENT, Ordering::AcqRel, Ordering::Acquire).is_err() {
            return;
        }
        let code = if close_code_valid(code) { code } else { CLOSE_NORMAL };
        let mut p = Vec::from(code.to_be_bytes());
        let mut r = reason;
        while r.len() > 123 {
            let mut k = 123;
            while !r.is_char_boundary(k) {
                k -= 1;
            }
            r = &r[..k];
        }
        p.extend_from_slice(r.as_bytes());
        if self.write_frame(OP_CLOSE, &p).is_err() {
            self.state.store(CLOSED, Ordering::Release);
            return;
        }
        // the peer's close ends it; a reader on another thread takes it itself
        if let Some(mut rd) = self.rd.try_lock() {
            let end = plat::time_us() + 1_000_000;
            while rd.end.is_none() && plat::time_us() < end {
                match rd.dec.next() {
                    Ok(Some(Frame::Close(c))) => {
                        let (code, reason) = c.unwrap_or((CLOSE_NO_STATUS, String::new()));
                        rd.end = Some(WsError::Closed(code, reason));
                        break;
                    }
                    Ok(Some(_)) => continue,
                    Err(_) => break,
                    Ok(None) => {}
                }
                if !self.s.wait_readable(100) {
                    continue;
                }
                let ReadSide { scratch, dec, .. } = &mut *rd;
                match self.s.read(scratch) {
                    Ok(n) if n > 0 => {
                        if dec.feed(&scratch[..n]).is_err() {
                            break;
                        }
                    }
                    _ => break,
                }
            }
            if rd.end.is_none() {
                rd.end = Some(WsError::Closed(code, String::from(r)));
            }
        }
        self.state.store(CLOSED, Ordering::Release);
        self.s.shutdown();
    }
}

impl Drop for WebSocket {
    fn drop(&mut self) {
        if self.state.load(Ordering::Acquire) == OPEN {
            self.close(CLOSE_NORMAL, "");
        }
    }
}

// ------------------------------------------------------------------ the server side

/// the server side's check of upgrade requests, shared by a Server's handler: limits for its
/// connections, the origins and subprotocols it takes, the count of open connections
pub struct Acceptor {
    lim: Limits,
    origins: Vec<String>,
    protocols: Vec<String>,
    live: Arc<AtomicUsize>,
}

impl Acceptor {
    /// connections with these limits (max_connections at once)
    pub fn new(lim: Limits) -> Acceptor {
        Acceptor { lim, origins: Vec::new(), protocols: Vec::new(), live: Arc::new(AtomicUsize::new(0)) }
    }

    /// take only requests with one of these Origin headers ("https://game.example.org"; a page
    /// on another site cannot open a connection with its visitor's cookies then).  Empty: any
    pub fn origins(mut self, list: &[&str]) -> Acceptor {
        self.origins = list.iter().map(|s| String::from(*s)).collect();
        self
    }

    /// the subprotocols this side speaks, preferred first; the first the client offers is chosen
    pub fn protocols(mut self, list: &[&str]) -> Acceptor {
        self.protocols = list.iter().map(|s| String::from(*s)).collect();
        self
    }

    /// connections open now
    pub fn connections(&self) -> usize {
        self.live.load(Ordering::Acquire)
    }

    /// answer an upgrade request: 101 with `on_open` running on a thread of the server's own
    /// once the answer is out (the connection ends when it returns), or 400 (not a WebSocket
    /// request), 403 (an origin not on the list), 426 (another version) or 503 (max_connections)
    pub fn accept(&self, req: &Request, on_open: impl FnOnce(WebSocket) + Send + 'static) -> Response {
        let h = &req.headers;
        if req.method != "GET" || !h.has_token("upgrade", "websocket") || !h.has_token("connection", "upgrade") {
            return Response::text(400, "not a WebSocket request");
        }
        if h.get("sec-websocket-version").map(str::trim) != Some("13") {
            return Response::text(426, "WebSocket version 13 only").header("Sec-WebSocket-Version", "13");
        }
        let Some(key) = h.get("sec-websocket-key").map(str::trim) else {
            return Response::text(400, "no Sec-WebSocket-Key");
        };
        if key.len() != 24 || crypto::b64::decode(key).map(|k| k.len()) != Some(16) {
            return Response::text(400, "bad Sec-WebSocket-Key");
        }
        if !self.origins.is_empty() && !h.get("origin").is_some_and(|o| self.origins.iter().any(|a| a.eq_ignore_ascii_case(o.trim()))) {
            return Response::text(403, "origin not allowed");
        }
        // a slot first: a burst of requests cannot pass the limit between check and count
        if self.live.fetch_add(1, Ordering::AcqRel) >= self.lim.max_connections {
            self.live.fetch_sub(1, Ordering::AcqRel);
            return Response::text(503, "too many connections");
        }
        let slot = Slot(self.live.clone());
        let offered: Vec<&str> = h.0.iter().filter(|(k, _)| k.eq_ignore_ascii_case("sec-websocket-protocol")).flat_map(|(_, v)| v.split(',')).map(str::trim).collect();
        let protocol = self.protocols.iter().find(|p| offered.contains(&p.as_str())).cloned();
        let mut r = Response::new(101);
        r.headers.set("Upgrade", "websocket");
        r.headers.set("Connection", "Upgrade");
        r.headers.set("Sec-WebSocket-Accept", &accept_key(key));
        if let Some(p) = &protocol {
            r.headers.set("Sec-WebSocket-Protocol", p);
        }
        let lim = self.lim;
        r.body = Body::Upgrade(Upgrade::new(move |up: Upgraded| {
            if let Ok(ws) = WebSocket::new(up.stream, Role::Server, lim, &up.buffered, Some(up.stop), up.peer, protocol, Some(slot)) {
                on_open(ws);
            }
        }));
        r
    }
}

/// Sec-WebSocket-Accept for a key: base64 of SHA-1 over the key and the GUID
pub fn accept_key(key: &str) -> String {
    let mut s = String::with_capacity(key.len() + GUID.len());
    s.push_str(key);
    s.push_str(GUID);
    crypto::b64::encode(&crypto::sha1::sha1(s.as_bytes()))
}

// ------------------------------------------------------------------ the client

/// how a client connects
#[derive(Clone)]
pub struct ClientConfig {
    /// the connection's limits (handshake_ms for the whole handshake)
    pub limits: Limits,
    /// the TCP connection within this long (the handshake has limits.handshake_ms in all)
    pub connect_ms: u32,
    /// subprotocols to offer (Sec-WebSocket-Protocol)
    pub protocols: Vec<String>,
    /// an Origin header (a browser always sends one; servers with an origin list check it)
    pub origin: Option<String>,
    /// further headers of the upgrade request (Authorization, cookies)
    pub headers: Headers,
    /// the TLS layer for wss:// (the same as Client::with_connector's)
    pub connector: Option<Connector>,
}

impl Default for ClientConfig {
    fn default() -> Self {
        ClientConfig { limits: Limits::default(), connect_ms: 5000, protocols: Vec::new(), origin: None, headers: Headers::new(), connector: None }
    }
}

/// a ws:// or wss:// URL as netio's Url (scheme http / https)
pub fn parse_url(url: &str) -> Option<Url> {
    let (scheme, rest) = url.split_once("://")?;
    let http = if scheme.eq_ignore_ascii_case("ws") {
        "http"
    } else if scheme.eq_ignore_ascii_case("wss") {
        "https"
    } else {
        return None;
    };
    Url::parse(&format!("{}://{}", http, rest))
}

/// connect to a WebSocket server: TCP (and TLS for wss:// through `cfg.connector`), the upgrade
/// request, the server's 101 with the right Sec-WebSocket-Accept, all within handshake_ms
pub fn connect(url: &str, cfg: &ClientConfig) -> Result<WebSocket, WsError> {
    let u = parse_url(url).ok_or_else(|| WsError::Url(String::from(url)))?;
    let lim = cfg.limits;
    let t0 = plat::time_us();
    let addrs = crate::dns::resolve(&u.host, u.port)?;
    let tcp = TcpStream::connect_any(&addrs, cfg.connect_ms.min(lim.handshake_ms))?;
    let _ = tcp.set_nodelay(true);
    let s: Box<dyn Stream> = if u.is_https() {
        let c = cfg.connector.as_ref().ok_or_else(|| WsError::Url(String::from("wss:// needs a TLS connector")))?;
        tcp.set_read_timeout(Some(lim.handshake_ms));
        tcp.set_write_timeout(Some(lim.handshake_ms));
        c(&u, tcp).map_err(|e| match e {
            crate::HttpError::Net(n) => WsError::Net(n),
            crate::HttpError::Tls(t) => WsError::Tls(t),
            other => WsError::Tls(format!("{}", other)),
        })?
    } else {
        Box::new(tcp)
    };
    let left = (lim.handshake_ms as u64 * 1000).saturating_sub(plat::time_us() - t0) / 1000;
    if left == 0 {
        return Err(WsError::TimedOut);
    }
    s.set_timeouts(Some(left as u32), Some(left as u32));
    let mut kb = [0u8; 16];
    plat::fill_random(&mut kb);
    let key = crypto::b64::encode(&kb);
    let mut h = Headers::new();
    let default_port = if u.is_https() { 443 } else { 80 };
    let host = if u.host.contains(':') { format!("[{}]", u.host) } else { u.host.clone() };
    h.set("Host", &if u.port == default_port { host } else { format!("{}:{}", host, u.port) });
    h.set("Upgrade", "websocket");
    h.set("Connection", "Upgrade");
    h.set("Sec-WebSocket-Key", &key);
    h.set("Sec-WebSocket-Version", "13");
    h.set("User-Agent", "JFLib-netio");
    if let Some(o) = &cfg.origin {
        h.set("Origin", o);
    }
    if !cfg.protocols.is_empty() {
        h.set("Sec-WebSocket-Protocol", &cfg.protocols.join(", "));
    }
    for (k, v) in &cfg.headers.0 {
        h.set(k, v);
    }
    if !http::write_request(&*s, "GET", &u.target, &h, &[]) {
        return Err(WsError::Net(NetError::new(NetErrorKind::Reset, 0)));
    }
    let mut rd = Reader::new(&*s);
    rd.set_deadline(Some(t0 + lim.handshake_ms as u64 * 1000));
    let (start, headers) = match rd.head() {
        Ok(x) => x,
        Err(http::ReadError::Net(e)) if e.kind == NetErrorKind::TimedOut => return Err(WsError::TimedOut),
        Err(http::ReadError::Net(e)) => return Err(WsError::Net(e)),
        Err(_) => return Err(WsError::Handshake(String::from("malformed answer"))),
    };
    let mut parts = start.splitn(3, ' ');
    let (version, status) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
    if !version.starts_with("HTTP/1.") || status != "101" {
        return Err(WsError::Handshake(format!("status {}", status)));
    }
    if !headers.has_token("upgrade", "websocket") || !headers.has_token("connection", "upgrade") {
        return Err(WsError::Handshake(String::from("no Upgrade: websocket")));
    }
    if headers.get("sec-websocket-accept").map(str::trim) != Some(accept_key(&key).as_str()) {
        return Err(WsError::Handshake(String::from("wrong Sec-WebSocket-Accept")));
    }
    if headers.get("sec-websocket-extensions").is_some() {
        return Err(WsError::Handshake(String::from("an extension nobody asked for")));
    }
    let protocol = headers.get("sec-websocket-protocol").map(|p| String::from(p.trim()));
    if let Some(p) = &protocol {
        if !cfg.protocols.iter().any(|o| o == p) {
            return Err(WsError::Handshake(format!("subprotocol {} was not offered", p)));
        }
    }
    let buffered = rd.take_buffered();
    drop(rd);
    let peer = addrs.first().copied();
    WebSocket::new(s, Role::Client, lim, &buffered, None, peer, protocol, None)
}
