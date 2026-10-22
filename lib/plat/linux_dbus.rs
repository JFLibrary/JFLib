// linux_dbus -- a small bus connection for linux_tray.rs, linux_ibus.rs, linux_atspi.rs,
// linux_life.rs and the desktop integration (linux_desk.rs: power, taskbar, autostart; the
// portal dialogs keep their own client in linux_portal.rs):
// $DBUS_SESSION_BUS_ADDRESS (unix:path= / unix:abstract=, else /run/user/<uid>/bus), the system
// bus or another bus address, SASL EXTERNAL, Hello; little-endian messages built from and
// parsed into Value trees; method calls, returns, errors and signals; file descriptors are sent
// (Msg::fds, Value::H; a connection made with Conn::session_fds), never received.  One owner at
// a time (its thread).
//
//   Conn::session() -> Option<Conn>     connected and named (Conn::unique)
//   Conn::session_fds() -> Option<Conn> the same with descriptor passing negotiated
//                                       (Conn::fds_ok: the bus agreed)
//   Conn::connect(address) -> Option<Conn>   the same for a bus address (the AT-SPI2 bus)
//   Conn::system() -> Option<Conn>      the same on the system bus (logind; linux_life.rs)
//   Conn::send(&Msg) -> serial          Conn::read(timeout_ms) -> Option<Msg> (None: nothing yet
//                                       or the connection broke: Conn::broken)
//   Msg::call / Msg::reply / Msg::error / Msg::signal   the four message kinds
//   Value::{Y, B, I, U, X, D, S, O, G, V, A, R, E}      byte, bool, int32, uint32, int64 (sent
//                                       only: a received x / t reads as U), double, string,
//                                       object path, signature, variant, array (element
//                                       signature), struct, dict entry

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;

use rt::os::syscall6;

const SYS_READ: usize = 0;
const SYS_WRITE: usize = 1;
const SYS_CLOSE: usize = 3;
const SYS_POLL: usize = 7;
const SYS_SOCKET: usize = 41;
const SYS_CONNECT: usize = 42;
const SYS_SENDMSG: usize = 46;
const SYS_GETUID: usize = 102;
const MSG_NOSIGNAL: usize = 0x4000;
const MSG_MAX: usize = 1 << 20;

pub(crate) const T_CALL: u8 = 1;
pub(crate) const T_RETURN: u8 = 2;
pub(crate) const T_ERROR: u8 = 3;
pub(crate) const T_SIGNAL: u8 = 4;
/// a call that wants no answer
pub(crate) const F_NO_REPLY: u8 = 1;

/// a D-Bus value
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Value {
    Y(u8),
    B(bool),
    I(i32),
    U(u32),
    /// int64 (only sent: received 64-bit integers are read as U)
    X(i64),
    /// a file descriptor: the index into the message's fds (only sent: received ones read as U)
    H(u32),
    D(f64),
    S(String),
    O(String),
    G(String),
    V(Box<Value>),
    /// element signature, elements
    A(String, Vec<Value>),
    R(Vec<Value>),
    E(Box<Value>, Box<Value>),
}

impl Value {
    pub(crate) fn sig(&self) -> String {
        match self {
            Value::Y(_) => String::from("y"),
            Value::B(_) => String::from("b"),
            Value::I(_) => String::from("i"),
            Value::U(_) => String::from("u"),
            Value::X(_) => String::from("x"),
            Value::H(_) => String::from("h"),
            Value::D(_) => String::from("d"),
            Value::S(_) => String::from("s"),
            Value::O(_) => String::from("o"),
            Value::G(_) => String::from("g"),
            Value::V(_) => String::from("v"),
            Value::A(e, _) => alloc::format!("a{e}"),
            Value::R(f) => alloc::format!("({})", f.iter().map(|v| v.sig()).collect::<String>()),
            Value::E(k, v) => alloc::format!("{{{}{}}}", k.sig(), v.sig()),
        }
    }

    pub(crate) fn str(s: &str) -> Value {
        Value::S(String::from(s))
    }

    pub(crate) fn var(v: Value) -> Value {
        Value::V(Box::new(v))
    }

    /// a{sv} from (key, value) pairs
    pub(crate) fn dict(pairs: Vec<(&str, Value)>) -> Value {
        Value::A(String::from("{sv}"), pairs.into_iter().map(|(k, v)| Value::E(Box::new(Value::str(k)), Box::new(Value::var(v)))).collect())
    }

    pub(crate) fn as_str(&self) -> Option<&str> {
        match self {
            Value::S(s) | Value::O(s) | Value::G(s) => Some(s),
            _ => None,
        }
    }

    pub(crate) fn as_u32(&self) -> Option<u32> {
        match self {
            Value::U(v) => Some(*v),
            Value::I(v) => Some(*v as u32),
            Value::Y(v) => Some(*v as u32),
            Value::B(b) => Some(*b as u32),
            Value::V(v) => v.as_u32(),
            _ => None,
        }
    }
}

/// one message
#[derive(Clone, Debug, Default)]
pub(crate) struct Msg {
    pub(crate) typ: u8,
    pub(crate) flags: u8,
    pub(crate) serial: u32,
    pub(crate) reply_to: u32,
    pub(crate) path: String,
    pub(crate) iface: String,
    pub(crate) member: String,
    pub(crate) error: String,
    pub(crate) dest: String,
    pub(crate) sender: String,
    pub(crate) body: Vec<Value>,
    /// descriptors sent with the message (Value::H indexes them; the caller keeps them open)
    pub(crate) fds: Vec<i32>,
}

impl Msg {
    pub(crate) fn call(dest: &str, path: &str, iface: &str, member: &str, body: Vec<Value>) -> Msg {
        Msg { typ: T_CALL, dest: String::from(dest), path: String::from(path), iface: String::from(iface), member: String::from(member), body, ..Msg::default() }
    }

    pub(crate) fn signal(path: &str, iface: &str, member: &str, body: Vec<Value>) -> Msg {
        Msg { typ: T_SIGNAL, path: String::from(path), iface: String::from(iface), member: String::from(member), body, ..Msg::default() }
    }

    /// the answer to the call m
    pub(crate) fn reply(m: &Msg, body: Vec<Value>) -> Msg {
        Msg { typ: T_RETURN, flags: F_NO_REPLY, reply_to: m.serial, dest: m.sender.clone(), body, ..Msg::default() }
    }

    /// an error answer to the call m
    pub(crate) fn error(m: &Msg, name: &str, text: &str) -> Msg {
        Msg { typ: T_ERROR, flags: F_NO_REPLY, reply_to: m.serial, dest: m.sender.clone(), error: String::from(name), body: alloc::vec![Value::str(text)], ..Msg::default() }
    }
}

// ---------------------------------------------------------------- marshalling

fn align_of(c: u8) -> usize {
    match c {
        b'y' | b'g' | b'v' => 1,
        b'n' | b'q' => 2,
        b'x' | b't' | b'd' | b'(' | b'{' => 8,
        _ => 4,
    }
}

fn pad(b: &mut Vec<u8>, n: usize) {
    while b.len() % n != 0 {
        b.push(0);
    }
}

fn put(b: &mut Vec<u8>, v: &Value) {
    match v {
        Value::Y(x) => b.push(*x),
        Value::B(x) => {
            pad(b, 4);
            b.extend_from_slice(&(*x as u32).to_le_bytes());
        }
        Value::I(x) => {
            pad(b, 4);
            b.extend_from_slice(&x.to_le_bytes());
        }
        Value::U(x) | Value::H(x) => {
            pad(b, 4);
            b.extend_from_slice(&x.to_le_bytes());
        }
        Value::X(x) => {
            pad(b, 8);
            b.extend_from_slice(&x.to_le_bytes());
        }
        Value::D(x) => {
            pad(b, 8);
            b.extend_from_slice(&x.to_le_bytes());
        }
        Value::S(s) | Value::O(s) => {
            pad(b, 4);
            b.extend_from_slice(&(s.len() as u32).to_le_bytes());
            b.extend_from_slice(s.as_bytes());
            b.push(0);
        }
        Value::G(s) => {
            b.push(s.len() as u8);
            b.extend_from_slice(s.as_bytes());
            b.push(0);
        }
        Value::V(inner) => {
            put(b, &Value::G(inner.sig()));
            put(b, inner);
        }
        Value::A(elem, items) => {
            pad(b, 4);
            let at = b.len();
            b.extend_from_slice(&[0; 4]);
            pad(b, align_of(elem.as_bytes().first().copied().unwrap_or(b'y')));
            let start = b.len();
            for it in items {
                put(b, it);
            }
            let n = (b.len() - start) as u32;
            b[at..at + 4].copy_from_slice(&n.to_le_bytes());
        }
        Value::R(fields) => {
            pad(b, 8);
            for f in fields {
                put(b, f);
            }
        }
        Value::E(k, v) => {
            pad(b, 8);
            put(b, k);
            put(b, v);
        }
    }
}

// the end of the complete type that starts at sig[i]
fn type_end(sig: &[u8], i: usize) -> Option<usize> {
    match *sig.get(i)? {
        b'a' => type_end(sig, i + 1),
        b'(' | b'{' => {
            let mut j = i + 1;
            while *sig.get(j)? != b')' && sig[j] != b'}' {
                j = type_end(sig, j)?;
            }
            Some(j + 1)
        }
        _ => Some(i + 1),
    }
}

struct Rd<'a> {
    b: &'a [u8],
    p: usize,
    depth: u32,
}

impl Rd<'_> {
    fn align(&mut self, n: usize) -> Option<()> {
        self.p = self.p.checked_add((n - self.p % n) % n)?;
        (self.p <= self.b.len()).then_some(())
    }

    fn take(&mut self, n: usize) -> Option<&[u8]> {
        let s = self.b.get(self.p..self.p.checked_add(n)?)?;
        self.p += n;
        Some(s)
    }

    fn u32(&mut self) -> Option<u32> {
        self.align(4)?;
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }

    fn string(&mut self) -> Option<String> {
        let n = self.u32()? as usize;
        let s = String::from(core::str::from_utf8(self.take(n)?).ok()?);
        self.take(1)?;
        Some(s)
    }

    fn sig(&mut self) -> Option<String> {
        let n = *self.take(1)?.first()? as usize;
        let s = String::from(core::str::from_utf8(self.take(n)?).ok()?);
        self.take(1)?;
        Some(s)
    }

    // one value of the complete type sig
    fn value(&mut self, sig: &[u8]) -> Option<Value> {
        self.depth += 1;
        if self.depth > 32 {
            return None;
        }
        let v = match sig[0] {
            b'y' => Value::Y(*self.take(1)?.first()?),
            b'b' => Value::B(self.u32()? != 0),
            b'i' => Value::I(self.u32()? as i32),
            b'u' => Value::U(self.u32()?),
            b'n' | b'q' => {
                self.align(2)?;
                let x = u16::from_le_bytes(self.take(2)?.try_into().ok()?);
                Value::U(x as u32)
            }
            b'x' | b't' => {
                self.align(8)?;
                let x = u64::from_le_bytes(self.take(8)?.try_into().ok()?);
                Value::U(x as u32)
            }
            b'd' => {
                self.align(8)?;
                Value::D(f64::from_le_bytes(self.take(8)?.try_into().ok()?))
            }
            b'h' => Value::U(self.u32()?),
            b's' => Value::S(self.string()?),
            b'o' => Value::O(self.string()?),
            b'g' => Value::G(self.sig()?),
            b'v' => {
                let s = self.sig()?;
                if type_end(s.as_bytes(), 0)? != s.len() {
                    return None;
                }
                Value::V(Box::new(self.value(s.as_bytes())?))
            }
            b'a' => {
                let n = self.u32()? as usize;
                let elem = &sig[1..type_end(sig, 1)?];
                self.align(align_of(elem[0]))?;
                let end = self.p.checked_add(n)?;
                if end > self.b.len() {
                    return None;
                }
                let mut items = Vec::new();
                while self.p < end {
                    items.push(self.value(elem)?);
                }
                Value::A(String::from(core::str::from_utf8(elem).ok()?), items)
            }
            b'(' | b'{' => {
                self.align(8)?;
                let close = type_end(sig, 0)? - 1;
                let mut j = 1;
                let mut fields = Vec::new();
                while j < close {
                    let e = type_end(sig, j)?;
                    fields.push(self.value(&sig[j..e])?);
                    j = e;
                }
                if sig[0] == b'{' {
                    let mut it = fields.into_iter();
                    let (k, v) = (it.next()?, it.next()?);
                    Value::E(Box::new(k), Box::new(v))
                } else {
                    Value::R(fields)
                }
            }
            _ => return None,
        };
        self.depth -= 1;
        Some(v)
    }
}

fn encode(m: &Msg, serial: u32) -> Vec<u8> {
    let mut body = Vec::new();
    for v in &m.body {
        put(&mut body, v);
    }
    let sig: String = m.body.iter().map(|v| v.sig()).collect();
    let mut fields: Vec<Value> = Vec::new();
    let mut field = |code: u8, v: Value| fields.push(Value::R(alloc::vec![Value::Y(code), Value::var(v)]));
    if !m.path.is_empty() {
        field(1, Value::O(m.path.clone()));
    }
    if !m.iface.is_empty() {
        field(2, Value::S(m.iface.clone()));
    }
    if !m.member.is_empty() {
        field(3, Value::S(m.member.clone()));
    }
    if !m.error.is_empty() {
        field(4, Value::S(m.error.clone()));
    }
    if m.reply_to != 0 {
        field(5, Value::U(m.reply_to));
    }
    if !m.dest.is_empty() {
        field(6, Value::S(m.dest.clone()));
    }
    if !sig.is_empty() {
        field(8, Value::G(sig));
    }
    if !m.fds.is_empty() {
        field(9, Value::U(m.fds.len() as u32));
    }
    let mut out = alloc::vec![b'l', m.typ, m.flags, 1];
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(&serial.to_le_bytes());
    put(&mut out, &Value::A(String::from("(yv)"), fields));
    pad(&mut out, 8);
    out.extend_from_slice(&body);
    out
}

// one message from the start of b -> (message, its length); None: incomplete or malformed
// (Err: malformed, the connection must close)
fn decode(b: &[u8]) -> Result<Option<(Msg, usize)>, ()> {
    if b.len() < 16 {
        return Ok(None);
    }
    if b[0] != b'l' || b[3] != 1 {
        return Err(());
    }
    let body_len = u32::from_le_bytes(b[4..8].try_into().map_err(|_| ())?) as usize;
    let fields_len = u32::from_le_bytes(b[12..16].try_into().map_err(|_| ())?) as usize;
    let head = 16 + fields_len;
    let total = head + (8 - head % 8) % 8 + body_len;
    if body_len > MSG_MAX || fields_len > MSG_MAX {
        return Err(());
    }
    if b.len() < total {
        return Ok(None);
    }
    let mut m = Msg { typ: b[1], flags: b[2], serial: u32::from_le_bytes(b[8..12].try_into().map_err(|_| ())?), ..Msg::default() };
    let mut r = Rd { b: &b[..head], p: 12, depth: 0 };
    let Some(Value::A(_, fields)) = r.value(b"a(yv)") else { return Err(()) };
    let mut sig = String::new();
    for f in fields {
        let Value::R(pair) = f else { continue };
        let (Some(Value::Y(code)), Some(Value::V(v))) = (pair.first(), pair.get(1)) else { continue };
        let s = v.as_str().map(String::from).unwrap_or_default();
        match code {
            1 => m.path = s,
            2 => m.iface = s,
            3 => m.member = s,
            4 => m.error = s,
            5 => m.reply_to = v.as_u32().unwrap_or(0),
            6 => m.dest = s,
            7 => m.sender = s,
            8 => sig = s,
            _ => {}
        }
    }
    let start = head + (8 - head % 8) % 8;
    let mut r = Rd { b: &b[start..total], p: 0, depth: 0 };
    let sb = sig.as_bytes();
    let mut i = 0;
    while i < sb.len() {
        let e = type_end(sb, i).ok_or(())?;
        m.body.push(r.value(&sb[i..e]).ok_or(())?);
        i = e;
    }
    Ok(Some((m, total)))
}

// ---------------------------------------------------------------- the connection

/// a session-bus connection
pub(crate) struct Conn {
    fd: i32,
    serial: u32,
    rbuf: Vec<u8>,
    pub(crate) unique: String,
    pub(crate) broken: bool,
    /// the bus agreed to descriptor passing (Conn::session_fds)
    pub(crate) fds_ok: bool,
}

impl Drop for Conn {
    fn drop(&mut self) {
        // SAFETY: our socket.
        unsafe { syscall6(SYS_CLOSE, self.fd as usize, 0, 0, 0, 0, 0) };
    }
}

// (path bytes, abstract) of the first unix: entry of a bus address
fn parse_address(a: &str) -> Option<(Vec<u8>, bool)> {
    for part in a.split(';') {
        let Some(rest) = part.strip_prefix("unix:") else { continue };
        for kv in rest.split(',') {
            if let Some(p) = kv.strip_prefix("path=") {
                return Some((p.as_bytes().to_vec(), false));
            }
            if let Some(p) = kv.strip_prefix("abstract=") {
                return Some((p.as_bytes().to_vec(), true));
            }
        }
    }
    None
}

// (path bytes, abstract) of the session bus
fn bus_address() -> Option<(Vec<u8>, bool)> {
    if let Some(a) = crate::env("DBUS_SESSION_BUS_ADDRESS") {
        return parse_address(&a);
    }
    // SAFETY: getuid has no arguments.
    let uid = unsafe { syscall6(SYS_GETUID, 0, 0, 0, 0, 0, 0) };
    let p = alloc::format!("/run/user/{uid}/bus");
    (crate::file_exists_raw(&p) != 0).then(|| (p.into_bytes(), false))
}

// the system bus: DBUS_SYSTEM_BUS_ADDRESS, else the standard socket
fn system_address() -> Option<(Vec<u8>, bool)> {
    if let Some(a) = crate::env("DBUS_SYSTEM_BUS_ADDRESS") {
        return parse_address(&a);
    }
    let p = "/run/dbus/system_bus_socket";
    (crate::file_exists_raw(p) != 0).then(|| (p.as_bytes().to_vec(), false))
}

/// whether a session bus is announced
pub(crate) fn session_available() -> bool {
    bus_address().is_some()
}

impl Conn {
    /// connect to the session bus, authenticate, say Hello
    pub(crate) fn session() -> Option<Conn> {
        let (path, abstract_) = bus_address()?;
        Conn::open(&path, abstract_, false)
    }

    /// the session bus with descriptor passing (NEGOTIATE_UNIX_FD; fds_ok when it agreed)
    pub(crate) fn session_fds() -> Option<Conn> {
        let (path, abstract_) = bus_address()?;
        Conn::open(&path, abstract_, true)
    }

    /// the same on the system bus (logind)
    pub(crate) fn system() -> Option<Conn> {
        let (path, abstract_) = system_address()?;
        Conn::open(&path, abstract_, false)
    }

    /// the same for a bus address ("unix:path=..." / "unix:abstract=...")
    pub(crate) fn connect(address: &str) -> Option<Conn> {
        let (path, abstract_) = parse_address(address)?;
        Conn::open(&path, abstract_, false)
    }

    fn open(path: &[u8], abstract_: bool, fds: bool) -> Option<Conn> {
        if path.len() + 1 >= 108 {
            return None;
        }
        let mut addr = [0u8; 110];
        addr[..2].copy_from_slice(&1u16.to_ne_bytes());
        let off = if abstract_ { 3 } else { 2 };
        addr[off..off + path.len()].copy_from_slice(path);
        let len = off + path.len() + if abstract_ { 0 } else { 1 };
        // SAFETY: a close-on-exec Unix stream socket and a valid address.
        let fd = unsafe { syscall6(SYS_SOCKET, 1, 1 | 0x80000, 0, 0, 0, 0) };
        if fd < 0 {
            return None;
        }
        let mut c = Conn { fd: fd as i32, serial: 0, rbuf: Vec::new(), unique: String::new(), broken: false, fds_ok: false };
        // SAFETY: the address record lives during the call.
        if unsafe { syscall6(SYS_CONNECT, fd as usize, addr.as_ptr() as usize, len, 0, 0, 0) } < 0 {
            return None;
        }
        // SASL EXTERNAL with the uid in hex
        // SAFETY: getuid has no arguments.
        let uid = alloc::format!("{}", unsafe { syscall6(SYS_GETUID, 0, 0, 0, 0, 0, 0) });
        let hex: String = uid.bytes().map(|b| alloc::format!("{b:02x}")).collect();
        if !c.write_all(&[0]) || !c.write_all(alloc::format!("AUTH EXTERNAL {hex}\r\n").as_bytes()) {
            return None;
        }
        let line = c.read_line(3000)?;
        if !line.starts_with(b"OK ") {
            return None;
        }
        if fds {
            if !c.write_all(b"NEGOTIATE_UNIX_FD\r\n") {
                return None;
            }
            c.fds_ok = c.read_line(3000)?.starts_with(b"AGREE_UNIX_FD");
        }
        if !c.write_all(b"BEGIN\r\n") {
            return None;
        }
        let hello = c.send(&Msg::call("org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus", "Hello", Vec::new()))?;
        let r = c.wait_reply(hello, 3000)?;
        c.unique = r.body.first().and_then(|v| v.as_str()).map(String::from)?;
        Some(c)
    }

    pub(crate) fn fd(&self) -> i32 {
        self.fd
    }

    fn write_all(&mut self, mut b: &[u8]) -> bool {
        while !b.is_empty() {
            // SAFETY: a valid slice on our socket.
            let n = unsafe { syscall6(SYS_WRITE, self.fd as usize, b.as_ptr() as usize, b.len(), 0, 0, 0) };
            if n == -4 {
                continue;
            }
            if n <= 0 {
                self.broken = true;
                return false;
            }
            b = &b[n as usize..];
        }
        true
    }

    // wait up to ms for readable data -> more bytes arrived
    fn fill(&mut self, ms: i32) -> bool {
        let mut pfd = [self.fd as u32 as u64 | (1u64 << 32)]; // struct pollfd {fd, events POLLIN, revents}
        // SAFETY: one pollfd record.
        let r = unsafe { syscall6(SYS_POLL, pfd.as_mut_ptr() as usize, 1, ms as usize, 0, 0, 0) };
        if r <= 0 {
            return false;
        }
        let mut buf = [0u8; 8192];
        // SAFETY: a writable buffer.
        let n = unsafe { syscall6(SYS_READ, self.fd as usize, buf.as_mut_ptr() as usize, buf.len(), 0, 0, 0) };
        if n <= 0 {
            if n != -4 && n != -11 {
                self.broken = true;
            }
            return false;
        }
        self.rbuf.extend_from_slice(&buf[..n as usize]);
        true
    }

    fn read_line(&mut self, ms: i32) -> Option<Vec<u8>> {
        loop {
            if let Some(k) = self.rbuf.windows(2).position(|w| w == b"\r\n") {
                let line = self.rbuf[..k].to_vec();
                self.rbuf.drain(..k + 2);
                return Some(line);
            }
            if !self.fill(ms) {
                return None;
            }
        }
    }

    /// send a message -> its serial (None: the connection broke, or descriptors without
    /// fds_ok)
    pub(crate) fn send(&mut self, m: &Msg) -> Option<u32> {
        if !m.fds.is_empty() && (!self.fds_ok || m.fds.len() > 16) {
            return None;
        }
        self.serial = self.serial.wrapping_add(1).max(1);
        let s = self.serial;
        let b = encode(m, s);
        if m.fds.is_empty() {
            return self.write_all(&b).then_some(s);
        }
        // the descriptors ride with the first bytes (SCM_RIGHTS), the rest follows plainly
        let iov = [b.as_ptr() as usize, b.len()];
        let mut ctl = [0u8; 16 + 16 * 4];
        let len = 16 + 4 * m.fds.len();
        ctl[..8].copy_from_slice(&(len as u64).to_le_bytes()); // cmsg_len = CMSG_LEN(4 n)
        ctl[8..12].copy_from_slice(&1u32.to_le_bytes()); // SOL_SOCKET
        ctl[12..16].copy_from_slice(&1u32.to_le_bytes()); // SCM_RIGHTS
        for (i, fd) in m.fds.iter().enumerate() {
            ctl[16 + 4 * i..20 + 4 * i].copy_from_slice(&fd.to_le_bytes());
        }
        // msghdr: name, namelen, iov, iovlen, control, controllen (CMSG_SPACE), flags
        let mh = [0usize, 0, iov.as_ptr() as usize, 1, ctl.as_ptr() as usize, (len + 7) & !7, 0];
        loop {
            // SAFETY: a msghdr over live buffers of this frame.
            let n = unsafe { syscall6(SYS_SENDMSG, self.fd as usize, mh.as_ptr() as usize, MSG_NOSIGNAL, 0, 0, 0) };
            if n == -4 {
                continue;
            }
            if n <= 0 {
                self.broken = true;
                return None;
            }
            return self.write_all(&b[n as usize..]).then_some(s);
        }
    }

    /// the next message, waiting up to ms (-1: as long as it takes)
    pub(crate) fn read(&mut self, ms: i32) -> Option<Msg> {
        loop {
            match decode(&self.rbuf) {
                Ok(Some((m, n))) => {
                    self.rbuf.drain(..n);
                    return Some(m);
                }
                Ok(None) => {}
                Err(()) => {
                    self.broken = true;
                    return None;
                }
            }
            if self.broken || !self.fill(ms) {
                return None;
            }
        }
    }

    /// the answer to serial (other messages meanwhile are dropped: only during set-up)
    pub(crate) fn wait_reply(&mut self, serial: u32, ms: i32) -> Option<Msg> {
        let end = crate::time_us() + ms as u64 * 1000;
        while crate::time_us() < end {
            let m = self.read(((end - crate::time_us()) / 1000) as i32 + 1)?;
            if (m.typ == T_RETURN || m.typ == T_ERROR) && m.reply_to == serial {
                return (m.typ == T_RETURN).then_some(m);
            }
        }
        None
    }
}

/// a{sv} / a(...) helpers for tests of the marshalling
#[allow(dead_code)]
pub(crate) fn roundtrip(m: &Msg) -> Option<Msg> {
    decode(&encode(m, 7)).ok().flatten().map(|(m, _)| m)
}
