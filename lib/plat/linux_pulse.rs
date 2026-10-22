// linux_pulse -- (linux_audio.rs) the PulseAudio native protocol: packets, tag structs, a
// client connection over the Unix socket (also pipewire-pulse and WSLg's server)
//
//   server_paths() -> [path]   $PULSE_SERVER (unix:PATH / PATH entries; tcp: is skipped),
//                              $XDG_RUNTIME_DIR/pulse/native, /run/user/<uid>/pulse/native,
//                              /var/run/pulse/native (system mode)
//   Conn::open(app) -> Conn    connected, AUTH with the cookie ($PULSE_COOKIE,
//                              $XDG_CONFIG_HOME/pulse/cookie, ~/.config/pulse/cookie,
//                              ~/.pulse-cookie; none: zeros, which servers with anonymous auth
//                              and pipewire-pulse accept), SET_CLIENT_NAME (application.name)
//   Conn::call(cmd, args) -> reply tag struct (Err: the server's error code, or -1 when the
//                              connection broke); other packets meanwhile go to `pending`
//   Conn::send(cmd, args), Conn::write_data(channel, bytes), Conn::read(ms) -> Packet
//   Ts (writer) / Rd (reader)  the tag struct: typed values, each after its tag byte
//
// Packets: a 20-byte descriptor of five big-endian u32 (length, channel, offset high / low,
// flags), then the payload.  Channel u32::MAX: a command (a tag struct: u32 command, u32 tag,
// the arguments); otherwise audio for that stream channel (memblock, seek relative).  No shared
// memory: AUTH does not offer it, so the server never sends SHM / memfd blocks or an srbchannel.
// We speak version 32 and use the server's when it is lower (>= 13 needed).

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use rt::os::syscall6;

const SYS_READ: usize = 0;
const SYS_WRITE: usize = 1;
const SYS_CLOSE: usize = 3;
const SYS_POLL: usize = 7;
const SYS_SOCKET: usize = 41;
const SYS_CONNECT: usize = 42;
const SYS_GETUID: usize = 102;

pub(crate) const VERSION: u32 = 32;
const PACKET_MAX: usize = 16 << 20;

// commands (pulsecore/native-common.h)
pub(crate) const CMD_ERROR: u32 = 0;
pub(crate) const CMD_REPLY: u32 = 2;
pub(crate) const CMD_CREATE_PLAYBACK_STREAM: u32 = 3;
pub(crate) const CMD_DELETE_PLAYBACK_STREAM: u32 = 4;
pub(crate) const CMD_AUTH: u32 = 8;
pub(crate) const CMD_SET_CLIENT_NAME: u32 = 9;
pub(crate) const CMD_DRAIN_PLAYBACK_STREAM: u32 = 12;
pub(crate) const CMD_GET_PLAYBACK_LATENCY: u32 = 14;
pub(crate) const CMD_GET_SERVER_INFO: u32 = 20;
pub(crate) const CMD_GET_SINK_INPUT_INFO_LIST: u32 = 30;
pub(crate) const CMD_CORK_PLAYBACK_STREAM: u32 = 41;
pub(crate) const CMD_FLUSH_PLAYBACK_STREAM: u32 = 42;
pub(crate) const CMD_REQUEST: u32 = 61;
pub(crate) const CMD_UNDERFLOW: u32 = 63;
pub(crate) const CMD_PLAYBACK_STREAM_KILLED: u32 = 64;
pub(crate) const CMD_PLAYBACK_STREAM_MOVED: u32 = 78;

// tag bytes (pulsecore/tagstruct.h)
const T_STRING: u8 = b't';
const T_STRING_NULL: u8 = b'N';
const T_U32: u8 = b'L';
const T_U8: u8 = b'B';
const T_S64: u8 = b'r';
const T_SAMPLE_SPEC: u8 = b'a';
const T_ARBITRARY: u8 = b'x';
const T_TRUE: u8 = b'1';
const T_FALSE: u8 = b'0';
const T_TIMEVAL: u8 = b'T';
const T_USEC: u8 = b'U';
const T_CHANNEL_MAP: u8 = b'm';
const T_CVOLUME: u8 = b'v';
const T_PROPLIST: u8 = b'P';
const T_FORMAT_INFO: u8 = b'f';

/// PA_SAMPLE_FLOAT32LE
pub(crate) const FORMAT_F32LE: u8 = 5;
/// PA_VOLUME_NORM
pub(crate) const VOLUME_NORM: u32 = 0x1_0000;

/// a tag struct being written
#[derive(Default)]
pub(crate) struct Ts(pub(crate) Vec<u8>);

impl Ts {
    pub(crate) fn u32(&mut self, v: u32) -> &mut Ts {
        self.0.push(T_U32);
        self.0.extend_from_slice(&v.to_be_bytes());
        self
    }
    pub(crate) fn u8(&mut self, v: u8) -> &mut Ts {
        self.0.push(T_U8);
        self.0.push(v);
        self
    }
    pub(crate) fn bool(&mut self, v: bool) -> &mut Ts {
        self.0.push(if v { T_TRUE } else { T_FALSE });
        self
    }
    pub(crate) fn str(&mut self, v: Option<&str>) -> &mut Ts {
        match v {
            None => self.0.push(T_STRING_NULL),
            Some(s) => {
                self.0.push(T_STRING);
                self.0.extend(s.bytes().filter(|&b| b != 0));
                self.0.push(0);
            }
        }
        self
    }
    pub(crate) fn arbitrary(&mut self, b: &[u8]) -> &mut Ts {
        self.0.push(T_ARBITRARY);
        self.0.extend_from_slice(&(b.len() as u32).to_be_bytes());
        self.0.extend_from_slice(b);
        self
    }
    pub(crate) fn sample_spec(&mut self, format: u8, channels: u8, rate: u32) -> &mut Ts {
        self.0.extend_from_slice(&[T_SAMPLE_SPEC, format, channels]);
        self.0.extend_from_slice(&rate.to_be_bytes());
        self
    }
    pub(crate) fn channel_map(&mut self, pos: &[u8]) -> &mut Ts {
        self.0.extend_from_slice(&[T_CHANNEL_MAP, pos.len() as u8]);
        self.0.extend_from_slice(pos);
        self
    }
    pub(crate) fn cvolume(&mut self, v: &[u32]) -> &mut Ts {
        self.0.extend_from_slice(&[T_CVOLUME, v.len() as u8]);
        for x in v {
            self.0.extend_from_slice(&x.to_be_bytes());
        }
        self
    }
    /// string properties (the values are stored with their NUL, as libpulse does)
    pub(crate) fn proplist(&mut self, props: &[(&str, &str)]) -> &mut Ts {
        self.0.push(T_PROPLIST);
        for (k, v) in props {
            let mut b: Vec<u8> = v.bytes().filter(|&b| b != 0).collect();
            b.push(0);
            self.str(Some(k)).u32(b.len() as u32).arbitrary(&b);
        }
        self.str(None)
    }
    pub(crate) fn timeval(&mut self, sec: u32, usec: u32) -> &mut Ts {
        self.0.push(T_TIMEVAL);
        self.0.extend_from_slice(&sec.to_be_bytes());
        self.0.extend_from_slice(&usec.to_be_bytes());
        self
    }
}

/// a tag struct being read
pub(crate) struct Rd<'a> {
    b: &'a [u8],
    p: usize,
}

impl<'a> Rd<'a> {
    pub(crate) fn new(b: &'a [u8]) -> Rd<'a> {
        Rd { b, p: 0 }
    }
    pub(crate) fn done(&self) -> bool {
        self.p >= self.b.len()
    }
    fn tag(&mut self, t: u8) -> Option<()> {
        (*self.b.get(self.p)? == t).then(|| self.p += 1)
    }
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let s = self.b.get(self.p..self.p.checked_add(n)?)?;
        self.p += n;
        Some(s)
    }
    fn be32(&mut self) -> Option<u32> {
        Some(u32::from_be_bytes(self.take(4)?.try_into().ok()?))
    }
    fn be64(&mut self) -> Option<u64> {
        Some(u64::from_be_bytes(self.take(8)?.try_into().ok()?))
    }
    pub(crate) fn u32(&mut self) -> Option<u32> {
        self.tag(T_U32)?;
        self.be32()
    }
    pub(crate) fn u8(&mut self) -> Option<u8> {
        self.tag(T_U8)?;
        Some(self.take(1)?[0])
    }
    pub(crate) fn s64(&mut self) -> Option<i64> {
        self.tag(T_S64)?;
        Some(self.be64()? as i64)
    }
    pub(crate) fn usec(&mut self) -> Option<u64> {
        self.tag(T_USEC)?;
        self.be64()
    }
    pub(crate) fn bool(&mut self) -> Option<bool> {
        match *self.b.get(self.p)? {
            T_TRUE => {
                self.p += 1;
                Some(true)
            }
            T_FALSE => {
                self.p += 1;
                Some(false)
            }
            _ => None,
        }
    }
    /// a string (None inside: the NULL string)
    pub(crate) fn str(&mut self) -> Option<Option<String>> {
        if self.tag(T_STRING_NULL).is_some() {
            return Some(None);
        }
        self.tag(T_STRING)?;
        let rest = self.b.get(self.p..)?;
        let n = rest.iter().position(|&c| c == 0)?;
        let s = String::from_utf8_lossy(&rest[..n]).into_owned();
        self.p += n + 1;
        Some(Some(s))
    }
    pub(crate) fn arbitrary(&mut self) -> Option<&'a [u8]> {
        self.tag(T_ARBITRARY)?;
        let n = self.be32()? as usize;
        self.take(n)
    }
    /// (format, channels, rate)
    pub(crate) fn sample_spec(&mut self) -> Option<(u8, u8, u32)> {
        self.tag(T_SAMPLE_SPEC)?;
        let h = self.take(2)?;
        Some((h[0], h[1], self.be32()?))
    }
    pub(crate) fn channel_map(&mut self) -> Option<Vec<u8>> {
        self.tag(T_CHANNEL_MAP)?;
        let n = self.take(1)?[0] as usize;
        Some(self.take(n)?.to_vec())
    }
    pub(crate) fn cvolume(&mut self) -> Option<Vec<u32>> {
        self.tag(T_CVOLUME)?;
        let n = self.take(1)?[0] as usize;
        (0..n).map(|_| self.be32()).collect()
    }
    pub(crate) fn timeval(&mut self) -> Option<(u32, u32)> {
        self.tag(T_TIMEVAL)?;
        Some((self.be32()?, self.be32()?))
    }
    /// the string properties (other values are skipped)
    pub(crate) fn proplist(&mut self) -> Option<Vec<(String, String)>> {
        self.tag(T_PROPLIST)?;
        let mut out = Vec::new();
        loop {
            let Some(k) = self.str()? else { return Some(out) };
            let n = self.u32()? as usize;
            let v = self.arbitrary()?;
            if v.len() != n {
                return None;
            }
            if v.last() == Some(&0) {
                out.push((k, String::from_utf8_lossy(&v[..n - 1]).into_owned()));
            }
        }
    }
    /// a format info (encoding, properties)
    pub(crate) fn format_info(&mut self) -> Option<(u8, Vec<(String, String)>)> {
        self.tag(T_FORMAT_INFO)?;
        Some((self.u8()?, self.proplist()?))
    }
}

/// one packet: a command's tag struct (channel u32::MAX) or audio
pub(crate) struct Packet {
    pub(crate) channel: u32,
    pub(crate) data: Vec<u8>,
}

impl Packet {
    /// a command packet -> (command, tag, the arguments)
    pub(crate) fn command(&self) -> Option<(u32, u32, Rd<'_>)> {
        if self.channel != u32::MAX {
            return None;
        }
        let mut r = Rd::new(&self.data);
        let cmd = r.u32()?;
        let tag = r.u32()?;
        Some((cmd, tag, r))
    }
}

/// the bytes of a packet: descriptor and payload
pub(crate) fn frame(channel: u32, payload: &[u8]) -> Vec<u8> {
    let mut b = Vec::with_capacity(20 + payload.len());
    for v in [payload.len() as u32, channel, 0, 0, 0] {
        b.extend_from_slice(&v.to_be_bytes());
    }
    b.extend_from_slice(payload);
    b
}

/// the first whole packet in b -> (packet, bytes used); Err: a broken stream
pub(crate) fn unframe(b: &[u8]) -> Result<Option<(Packet, usize)>, ()> {
    if b.len() < 20 {
        return Ok(None);
    }
    let w = |i: usize| u32::from_be_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
    let (len, channel, flags) = (w(0) as usize, w(4), w(16));
    if len > PACKET_MAX || flags & 0x8000_0000 != 0 {
        return Err(()); // too big, or shared memory we did not offer
    }
    if b.len() < 20 + len {
        return Ok(None);
    }
    Ok(Some((Packet { channel, data: b[20..20 + len].to_vec() }, 20 + len)))
}

/// a command's tag struct: command, tag, then the caller's arguments
pub(crate) fn command(cmd: u32, tag: u32, args: impl FnOnce(&mut Ts)) -> Vec<u8> {
    let mut t = Ts::default();
    t.u32(cmd).u32(tag);
    args(&mut t);
    frame(u32::MAX, &t.0)
}

/// the channel map of n channels in WAVE order (mono; front left / right; + centre; quad;
/// 5.0; 5.1; 6.1; 7.1)
pub(crate) fn channel_positions(n: u16) -> Vec<u8> {
    // MONO 0, FL 1, FR 2, FC 3, RC 4, RL 5, RR 6, LFE 7, SL 10, SR 11
    match n {
        1 => vec![0],
        2 => vec![1, 2],
        3 => vec![1, 2, 3],
        4 => vec![1, 2, 5, 6],
        5 => vec![1, 2, 3, 5, 6],
        6 => vec![1, 2, 3, 7, 5, 6],
        7 => vec![1, 2, 3, 7, 4, 10, 11],
        _ => vec![1, 2, 3, 7, 5, 6, 10, 11],
    }
}

// ---------------------------------------------------------------- where the server is

fn uid() -> usize {
    // SAFETY: getuid has no arguments.
    unsafe { syscall6(SYS_GETUID, 0, 0, 0, 0, 0, 0) as usize }
}

/// the Unix sockets to try, in order
pub(crate) fn server_paths() -> Vec<String> {
    let mut out = Vec::new();
    if let Some(s) = crate::env("PULSE_SERVER") {
        for e in s.split_whitespace() {
            // "{machine id}unix:/path": the prefix names the machine it is valid on
            let e = if e.starts_with('{') { e.split_once('}').map_or("", |x| x.1) } else { e };
            if let Some(p) = e.strip_prefix("unix:") {
                out.push(String::from(p));
            } else if e.starts_with('/') {
                out.push(String::from(e));
            }
        }
        return out;
    }
    match crate::env("XDG_RUNTIME_DIR") {
        Some(d) if d.starts_with('/') => out.push(alloc::format!("{d}/pulse/native")),
        _ => out.push(alloc::format!("/run/user/{}/pulse/native", uid())),
    }
    out.push(String::from("/var/run/pulse/native"));
    out
}

/// the authentication cookie (256 bytes; zeros when there is none)
pub(crate) fn cookie() -> Vec<u8> {
    let mut paths = Vec::new();
    if let Some(p) = crate::env("PULSE_COOKIE") {
        paths.push(p);
    }
    match crate::env("XDG_CONFIG_HOME") {
        Some(c) if c.starts_with('/') => paths.push(alloc::format!("{c}/pulse/cookie")),
        _ => {}
    }
    if let Some(h) = crate::env("HOME") {
        paths.push(alloc::format!("{h}/.config/pulse/cookie"));
        paths.push(alloc::format!("{h}/.pulse-cookie"));
    }
    for p in paths {
        if let Some(b) = crate::read_file_raw(&p) {
            if b.len() >= 256 {
                return b[..256].to_vec();
            }
        }
    }
    vec![0; 256]
}

// ---------------------------------------------------------------- the connection

/// a client connection
pub(crate) struct Conn {
    fd: i32,
    rbuf: Vec<u8>,
    next_tag: u32,
    /// the protocol version both speak
    pub(crate) version: u32,
    /// packets that arrived while waiting for a reply
    pub(crate) pending: Vec<Packet>,
    pub(crate) broken: bool,
}

impl Drop for Conn {
    fn drop(&mut self) {
        // SAFETY: our socket.
        unsafe { syscall6(SYS_CLOSE, self.fd as usize, 0, 0, 0, 0, 0) };
    }
}

/// a connected Unix stream socket to path
fn connect_unix(path: &str) -> Option<i32> {
    let p = path.as_bytes();
    if p.is_empty() || p.len() >= 108 {
        return None;
    }
    let mut addr = [0u8; 110];
    addr[..2].copy_from_slice(&1u16.to_ne_bytes());
    addr[2..2 + p.len()].copy_from_slice(p);
    // SAFETY: a close-on-exec Unix stream socket.
    let fd = unsafe { syscall6(SYS_SOCKET, 1, 1 | 0x80000, 0, 0, 0, 0) };
    if fd < 0 {
        return None;
    }
    // SAFETY: the address record lives during the call.
    if unsafe { syscall6(SYS_CONNECT, fd as usize, addr.as_ptr() as usize, 2 + p.len() + 1, 0, 0, 0) } < 0 {
        // SAFETY: the socket just made.
        unsafe { syscall6(SYS_CLOSE, fd as usize, 0, 0, 0, 0, 0) };
        return None;
    }
    Some(fd as i32)
}

/// whether a server answers on one of the sockets (connect only)
pub(crate) fn reachable() -> bool {
    server_paths().iter().any(|p| {
        connect_unix(p).map(|fd| {
            // SAFETY: the socket just made.
            unsafe { syscall6(SYS_CLOSE, fd as usize, 0, 0, 0, 0, 0) };
        }).is_some()
    })
}

impl Conn {
    /// connect, authenticate and name the client -> None: no server here
    pub(crate) fn open(app: &str) -> Option<Conn> {
        let fd = server_paths().iter().find_map(|p| connect_unix(p))?;
        let mut c = Conn { fd, rbuf: Vec::new(), next_tag: 0, version: VERSION, pending: Vec::new(), broken: false };
        let ck = cookie();
        let rep = c.call(CMD_AUTH, |t| {
            t.u32(VERSION).arbitrary(&ck);
        }).ok()?;
        let server = rep.args().u32()? & 0xFFFF;
        if server < 13 {
            return None;
        }
        c.version = server.min(VERSION);
        let props = [("application.name", app), ("application.process.binary", app)];
        c.call(CMD_SET_CLIENT_NAME, |t| {
            t.proplist(&props);
        }).ok()?;
        Some(c)
    }

    pub(crate) fn fd(&self) -> i32 {
        self.fd
    }

    fn write_all(&mut self, mut b: &[u8]) -> bool {
        while !b.is_empty() {
            // SAFETY: a valid slice on our socket.
            let n = unsafe { syscall6(SYS_WRITE, self.fd as usize, b.as_ptr() as usize, b.len(), 0, 0, 0) };
            if n == -4 || n == -11 {
                // interrupted, or the socket's buffer is full: wait until it takes more
                let mut pfd = [self.fd as u32 as u64 | (4u64 << 32)]; // POLLOUT
                // SAFETY: one pollfd record.
                unsafe { syscall6(SYS_POLL, pfd.as_mut_ptr() as usize, 1, 1000, 0, 0, 0) };
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

    /// send a command -> its tag
    pub(crate) fn send(&mut self, cmd: u32, args: impl FnOnce(&mut Ts)) -> Option<u32> {
        let tag = self.next_tag;
        self.next_tag = self.next_tag.wrapping_add(1) & 0x7FFF_FFFF;
        self.write_all(&command(cmd, tag, args)).then_some(tag)
    }

    /// audio for a stream channel (split into blocks of at most 64 KiB)
    pub(crate) fn write_data(&mut self, channel: u32, mut b: &[u8]) -> bool {
        while !b.is_empty() {
            let n = b.len().min(65_536);
            let mut head = Vec::with_capacity(20);
            for v in [n as u32, channel, 0, 0, 0] {
                head.extend_from_slice(&v.to_be_bytes());
            }
            if !self.write_all(&head) || !self.write_all(&b[..n]) {
                return false;
            }
            b = &b[n..];
        }
        true
    }

    /// wait up to ms for data -> more arrived
    fn fill(&mut self, ms: i32) -> bool {
        let mut pfd = [self.fd as u32 as u64 | (1u64 << 32)]; // POLLIN
        // SAFETY: one pollfd record.
        let r = unsafe { syscall6(SYS_POLL, pfd.as_mut_ptr() as usize, 1, ms as isize as usize, 0, 0, 0) };
        if r <= 0 {
            return false;
        }
        let mut buf = [0u8; 16384];
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

    /// the next packet, waiting up to ms (0: only what is there)
    pub(crate) fn read(&mut self, ms: i32) -> Option<Packet> {
        loop {
            match unframe(&self.rbuf) {
                Ok(Some((p, n))) => {
                    self.rbuf.drain(..n);
                    return Some(p);
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

    /// a command and its reply -> the reply's arguments; Err(code): the server's error,
    /// Err(-1): no answer in 5 s or a broken connection
    pub(crate) fn call(&mut self, cmd: u32, args: impl FnOnce(&mut Ts)) -> Result<Reply, i32> {
        let tag = self.send(cmd, args).ok_or(-1)?;
        let end = crate::time_us() + 5_000_000;
        while crate::time_us() < end {
            let Some(p) = self.read(((end - crate::time_us()) / 1000) as i32 + 1) else {
                if self.broken {
                    return Err(-1);
                }
                continue;
            };
            match p.command() {
                Some((CMD_REPLY, t, _)) if t == tag => return Ok(Reply(p.data)),
                Some((CMD_ERROR, t, mut r)) if t == tag => return Err(r.u32().map_or(-1, |e| e as i32)),
                _ => self.pending.push(p),
            }
        }
        Err(-1)
    }
}

/// a reply's bytes
pub(crate) struct Reply(Vec<u8>);

impl Reply {
    /// its arguments (after the command and the tag)
    pub(crate) fn args(&self) -> Rd<'_> {
        let mut r = Rd::new(&self.0);
        let _ = (r.u32(), r.u32());
        r
    }
}
