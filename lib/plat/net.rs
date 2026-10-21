// net -- TCP, UDP and Unix domain sockets (IPv4 and IPv6) with timeouts
//
//   IpAddr / SocketAddr: parse ("10.0.0.5:9443", "[::1]:80"), Display, loopback / unspecified
//   TcpListener::bind(&addr) / accept() / accept_timeout(ms) / local_addr()
//   TcpStream::connect(&addr, timeout_ms) / read / write / write_all / read_exact,
//             set_read_timeout / set_write_timeout (ms, None = block), set_nodelay,
//             shutdown_write, peer_addr / local_addr, wait_readable(ms)
//   UdpSocket::bind / send_to / recv_from(buf, timeout_ms)
//   UnixListener::bind(path) / accept() / accept_timeout(ms); connect_unix(path, timeout_ms)
//             -> UnixStream (the stream calls of TcpStream; addresses and nodelay do not apply)
//             Linux, and Windows 10 1803 and later (AF_UNIX)
//   resolve(host, port): numeric addresses, "localhost"; Windows also the system resolver
//             (GetAddrInfoW).  Linux names go through netio::dns (no libc here).
//   NetError: the kinds callers act on, with the OS code
//
// The app's permissions (policy.rs, net.connect / net.listen) are checked before connect,
// bind, send_to and a name lookup; Unix sockets follow the file rules (fs.write on the path).
// A refusal is NetErrorKind::Denied.
//
// Timeouts are readiness waits (WSAPoll / poll) before each call, never SO_RCVTIMEO: after a
// timed-out blocking receive Windows leaves the connection in an undefined state.  Sockets are
// not inherited by child processes; sends never raise SIGPIPE.  Every method takes &self, so
// one thread may read while another writes (Arc<TcpStream>).

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;
use core::sync::atomic::{AtomicU32, Ordering};

// ------------------------------------------------------------------ addresses

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum IpAddr {
    V4([u8; 4]),
    V6([u8; 16]),
}

impl IpAddr {
    pub const LOCALHOST_V4: IpAddr = IpAddr::V4([127, 0, 0, 1]);
    pub const UNSPECIFIED_V4: IpAddr = IpAddr::V4([0, 0, 0, 0]);
    pub const LOCALHOST_V6: IpAddr = IpAddr::V6([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]);

    pub fn is_loopback(&self) -> bool {
        match self {
            IpAddr::V4(a) => a[0] == 127,
            IpAddr::V6(a) => *a == [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1] || self.mapped_v4().is_some_and(|v| v.is_loopback()),
        }
    }

    pub fn is_unspecified(&self) -> bool {
        match self {
            IpAddr::V4(a) => *a == [0; 4],
            IpAddr::V6(a) => *a == [0; 16],
        }
    }

    /// the IPv4 address inside an IPv4-mapped IPv6 address (::ffff:a.b.c.d)
    pub fn mapped_v4(&self) -> Option<IpAddr> {
        match self {
            IpAddr::V6(a) if a[..10] == [0; 10] && a[10] == 0xff && a[11] == 0xff => Some(IpAddr::V4([a[12], a[13], a[14], a[15]])),
            _ => None,
        }
    }

    /// "192.168.1.2" or "fe80::1" (RFC 5952 shortening)
    pub fn parse(s: &str) -> Option<IpAddr> {
        if s.contains(':') {
            parse_v6(s).map(IpAddr::V6)
        } else {
            parse_v4(s).map(IpAddr::V4)
        }
    }
}

fn parse_v4(s: &str) -> Option<[u8; 4]> {
    let mut out = [0u8; 4];
    let mut n = 0;
    for part in s.split('.') {
        if n == 4 || part.is_empty() || part.len() > 3 || !part.bytes().all(|b| b.is_ascii_digit()) || (part.len() > 1 && part.starts_with('0')) {
            return None;
        }
        out[n] = part.parse::<u16>().ok().filter(|v| *v < 256)? as u8;
        n += 1;
    }
    (n == 4).then_some(out)
}

fn parse_v6(s: &str) -> Option<[u8; 16]> {
    let s = s.split('%').next()?; // a zone suffix is not kept
    let (head, tail) = match s.find("::") {
        Some(i) => (&s[..i], Some(&s[i + 2..])),
        None => (s, None),
    };
    let mut words: Vec<u16> = Vec::new();
    let mut tail_words: Vec<u16> = Vec::new();
    let parse_part = |p: &str, out: &mut Vec<u16>, last: bool| -> Option<()> {
        if p.is_empty() {
            return Some(());
        }
        let parts: Vec<&str> = p.split(':').collect();
        for (i, w) in parts.iter().enumerate() {
            if last && i == parts.len() - 1 && w.contains('.') {
                let v4 = parse_v4(w)?;
                out.push(u16::from_be_bytes([v4[0], v4[1]]));
                out.push(u16::from_be_bytes([v4[2], v4[3]]));
            } else {
                if w.is_empty() || w.len() > 4 {
                    return None;
                }
                out.push(u16::from_str_radix(w, 16).ok()?);
            }
        }
        Some(())
    };
    parse_part(head, &mut words, tail.is_none())?;
    if let Some(t) = tail {
        parse_part(t, &mut tail_words, true)?;
        if words.len() + tail_words.len() > 7 {
            return None;
        }
        words.resize(8 - tail_words.len(), 0);
        words.extend(tail_words);
    }
    if words.len() != 8 {
        return None;
    }
    let mut out = [0u8; 16];
    for (i, w) in words.iter().enumerate() {
        out[i * 2..i * 2 + 2].copy_from_slice(&w.to_be_bytes());
    }
    Some(out)
}

impl fmt::Display for IpAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IpAddr::V4(a) => write!(f, "{}.{}.{}.{}", a[0], a[1], a[2], a[3]),
            IpAddr::V6(a) => {
                let w: Vec<u16> = (0..8).map(|i| u16::from_be_bytes([a[i * 2], a[i * 2 + 1]])).collect();
                // the longest run of zero words (at least 2) becomes "::"
                let (mut best, mut best_len, mut i) = (8, 1, 0);
                while i < 8 {
                    if w[i] == 0 {
                        let s = i;
                        while i < 8 && w[i] == 0 {
                            i += 1;
                        }
                        if i - s > best_len {
                            best = s;
                            best_len = i - s;
                        }
                    } else {
                        i += 1;
                    }
                }
                let mut i = 0;
                while i < 8 {
                    if i == best {
                        f.write_str(if i == 0 { "::" } else { ":" })?;
                        i += best_len;
                        continue;
                    }
                    write!(f, "{:x}", w[i])?;
                    if i < 7 {
                        f.write_str(":")?;
                    }
                    i += 1;
                }
                Ok(())
            }
        }
    }
}

/// an IP address and a port
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SocketAddr {
    pub ip: IpAddr,
    pub port: u16,
}

impl SocketAddr {
    pub const fn new(ip: IpAddr, port: u16) -> SocketAddr {
        SocketAddr { ip, port }
    }

    /// "1.2.3.4:80", "[::1]:80"
    pub fn parse(s: &str) -> Option<SocketAddr> {
        let (host, port) = if let Some(rest) = s.strip_prefix('[') {
            let (h, p) = rest.split_once("]:")?;
            (h, p)
        } else {
            s.rsplit_once(':')?
        };
        if !s.starts_with('[') && host.contains(':') {
            return None;
        }
        Some(SocketAddr { ip: IpAddr::parse(host)?, port: port.parse().ok()? })
    }
}

impl fmt::Display for SocketAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.ip {
            IpAddr::V4(_) => write!(f, "{}:{}", self.ip, self.port),
            IpAddr::V6(_) => write!(f, "[{}]:{}", self.ip, self.port),
        }
    }
}

// ------------------------------------------------------------------ errors

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetErrorKind {
    TimedOut,
    Refused,
    Reset,
    AddrInUse,
    AddrNotAvailable,
    Unreachable,
    NotFound,
    /// the peer closed the connection before all bytes arrived (read_exact)
    Eof,
    /// the app's permissions (apps/NAME/PERMISSIONS) do not allow the address or name
    Denied,
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NetError {
    pub kind: NetErrorKind,
    /// the OS error code (0 when the library detected the condition)
    pub code: i32,
}

impl NetError {
    pub const fn new(kind: NetErrorKind, code: i32) -> NetError {
        NetError { kind, code }
    }
}

impl fmt::Display for NetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let what = match self.kind {
            NetErrorKind::TimedOut => "timed out",
            NetErrorKind::Refused => "connection refused",
            NetErrorKind::Reset => "connection reset",
            NetErrorKind::AddrInUse => "address in use",
            NetErrorKind::AddrNotAvailable => "address not available",
            NetErrorKind::Unreachable => "network unreachable",
            NetErrorKind::NotFound => "host not found",
            NetErrorKind::Eof => "connection closed",
            NetErrorKind::Denied => "not allowed by the app's permissions",
            NetErrorKind::Other => "network error",
        };
        if self.code != 0 { write!(f, "{} (os error {})", what, self.code) } else { f.write_str(what) }
    }
}

pub type NetResult<T> = Result<T, NetError>;

fn timed_out() -> NetError {
    NetError::new(NetErrorKind::TimedOut, 0)
}

fn denied() -> NetError {
    NetError::new(NetErrorKind::Denied, 0)
}

// ------------------------------------------------------------------ OS layer

#[cfg(windows)]
mod sys {
    use super::{NetError, NetErrorKind, NetResult};
    use core::sync::atomic::{AtomicU8, Ordering};

    pub type Raw = usize;
    const INVALID: Raw = usize::MAX;
    pub const AF_INET: i32 = 2;
    pub const AF_INET6: i32 = 23;
    pub const SOL_SOCKET: i32 = 0xffff;
    pub const SO_REUSEADDR: i32 = -5; // SO_EXCLUSIVEADDRUSE: no other process may share the port
    pub const SO_ERROR: i32 = 0x1007;
    pub const SO_KEEPALIVE: i32 = 8;
    pub const IPPROTO_IPV6: i32 = 41;
    pub const IPV6_V6ONLY: i32 = 27;
    pub const POLLIN: i16 = 0x100; // POLLRDNORM
    pub const POLLOUT: i16 = 0x10; // POLLWRNORM
    pub const SHUT_WR: i32 = 1;

    #[repr(C)]
    pub struct PollFd {
        pub fd: Raw,
        pub events: i16,
        pub revents: i16,
    }

    #[repr(C)]
    struct WsaData {
        b: [u8; 408],
    }

    #[repr(C)]
    struct AddrInfoW {
        flags: i32,
        family: i32,
        socktype: i32,
        protocol: i32,
        addrlen: usize,
        canonname: *mut u16,
        addr: *const u8,
        next: *mut AddrInfoW,
    }

    #[link(name = "ws2_32", kind = "raw-dylib")]
    unsafe extern "system" {
        fn WSAStartup(ver: u16, data: *mut WsaData) -> i32;
        fn WSAGetLastError() -> i32;
        fn WSASocketW(af: i32, ty: i32, proto: i32, info: usize, g: u32, flags: u32) -> Raw;
        fn closesocket(s: Raw) -> i32;
        fn bind(s: Raw, addr: *const u8, len: i32) -> i32;
        fn listen(s: Raw, backlog: i32) -> i32;
        fn accept(s: Raw, addr: *mut u8, len: *mut i32) -> Raw;
        fn connect(s: Raw, addr: *const u8, len: i32) -> i32;
        fn recv(s: Raw, buf: *mut u8, len: i32, flags: i32) -> i32;
        fn send(s: Raw, buf: *const u8, len: i32, flags: i32) -> i32;
        fn recvfrom(s: Raw, buf: *mut u8, len: i32, flags: i32, from: *mut u8, fromlen: *mut i32) -> i32;
        fn sendto(s: Raw, buf: *const u8, len: i32, flags: i32, to: *const u8, tolen: i32) -> i32;
        fn setsockopt(s: Raw, level: i32, name: i32, val: *const u8, len: i32) -> i32;
        fn getsockopt(s: Raw, level: i32, name: i32, val: *mut u8, len: *mut i32) -> i32;
        fn getsockname(s: Raw, addr: *mut u8, len: *mut i32) -> i32;
        fn getpeername(s: Raw, addr: *mut u8, len: *mut i32) -> i32;
        fn shutdown(s: Raw, how: i32) -> i32;
        fn ioctlsocket(s: Raw, cmd: i32, arg: *mut u32) -> i32;
        fn WSAPoll(fds: *mut PollFd, n: u32, timeout: i32) -> i32;
        fn GetAddrInfoW(node: *const u16, service: *const u16, hints: *const AddrInfoW, res: *mut *mut AddrInfoW) -> i32;
        fn FreeAddrInfoW(ai: *mut AddrInfoW);
    }

    static STARTED: AtomicU8 = AtomicU8::new(0);

    fn startup() {
        if STARTED.load(Ordering::Acquire) == 1 {
            return;
        }
        let mut d = WsaData { b: [0; 408] };
        // SAFETY: WSAStartup fills the WSADATA record; repeated calls are reference counted.
        unsafe { WSAStartup(0x0202, &mut d) };
        STARTED.store(1, Ordering::Release);
    }

    pub fn kind(c: i32) -> NetErrorKind {
        match c {
            10060 => NetErrorKind::TimedOut,
            10061 => NetErrorKind::Refused,
            10054 | 10053 | 10058 => NetErrorKind::Reset,
            10048 => NetErrorKind::AddrInUse,
            10049 => NetErrorKind::AddrNotAvailable,
            10051 | 10065 | 10050 => NetErrorKind::Unreachable,
            11001 | 11004 => NetErrorKind::NotFound,
            _ => NetErrorKind::Other,
        }
    }

    fn err() -> NetError {
        // SAFETY: reads the calling thread's last socket error.
        let c = unsafe { WSAGetLastError() };
        NetError::new(kind(c), c)
    }

    /// a status result (0 = success)
    fn st(r: i32) -> NetResult<()> {
        if r == 0 { Ok(()) } else { Err(err()) }
    }

    /// a byte-count result (SOCKET_ERROR = -1)
    fn cnt(r: i32) -> NetResult<usize> {
        if r < 0 { Err(err()) } else { Ok(r as usize) }
    }

    pub fn would_block(e: &NetError) -> bool {
        e.code == 10035 || e.code == 10036
    }

    pub fn socket(af: i32, ty: i32, proto: i32) -> NetResult<Raw> {
        startup();
        // WSA_FLAG_OVERLAPPED | WSA_FLAG_NO_HANDLE_INHERIT
        // SAFETY: plain socket creation.
        let s = unsafe { WSASocketW(af, ty, proto, 0, 0, 0x01 | 0x80) };
        if s == INVALID { Err(err()) } else { Ok(s) }
    }

    pub fn close(s: Raw) {
        // SAFETY: the caller owns the socket.
        unsafe { closesocket(s) };
    }

    pub fn set_nonblocking(s: Raw, on: bool) -> NetResult<()> {
        let mut v = on as u32;
        // SAFETY: FIONBIO with a valid u32.
        st(unsafe { ioctlsocket(s, 0x8004_667e_u32 as i32, &mut v) })
    }

    pub fn bind_to(s: Raw, a: &[u8]) -> NetResult<()> {
        // SAFETY: a is a complete sockaddr of a.len() bytes.
        st(unsafe { bind(s, a.as_ptr(), a.len() as i32) })
    }
    pub fn listen_on(s: Raw, n: i32) -> NetResult<()> {
        // SAFETY: plain call.
        st(unsafe { listen(s, n) })
    }
    pub fn accept_one(s: Raw, buf: &mut [u8; 128], len: &mut i32) -> NetResult<Raw> {
        // SAFETY: buf has room for any sockaddr; len holds its size.
        let r = unsafe { accept(s, buf.as_mut_ptr(), len) };
        if r == INVALID { Err(err()) } else { Ok(r) }
    }
    pub fn connect_to(s: Raw, a: &[u8]) -> NetResult<()> {
        // SAFETY: a is a complete sockaddr.
        st(unsafe { connect(s, a.as_ptr(), a.len() as i32) })
    }
    pub fn recv_into(s: Raw, b: &mut [u8]) -> NetResult<usize> {
        // SAFETY: b is writable for the given length.
        cnt(unsafe { recv(s, b.as_mut_ptr(), b.len().min(i32::MAX as usize) as i32, 0) })
    }
    pub fn send_from(s: Raw, b: &[u8]) -> NetResult<usize> {
        // SAFETY: b is readable for the given length.
        cnt(unsafe { send(s, b.as_ptr(), b.len().min(i32::MAX as usize) as i32, 0) })
    }
    pub fn recv_from(s: Raw, b: &mut [u8], from: &mut [u8; 128], len: &mut i32) -> NetResult<usize> {
        // SAFETY: valid buffers.
        cnt(unsafe { recvfrom(s, b.as_mut_ptr(), b.len().min(i32::MAX as usize) as i32, 0, from.as_mut_ptr(), len) })
    }
    pub fn send_to(s: Raw, b: &[u8], to: &[u8]) -> NetResult<usize> {
        // SAFETY: valid buffers.
        cnt(unsafe { sendto(s, b.as_ptr(), b.len().min(i32::MAX as usize) as i32, 0, to.as_ptr(), to.len() as i32) })
    }
    pub fn set_opt(s: Raw, level: i32, name: i32, v: i32) -> NetResult<()> {
        let b = v.to_ne_bytes();
        // SAFETY: a 4-byte option value.
        st(unsafe { setsockopt(s, level, name, b.as_ptr(), 4) })
    }
    pub fn get_opt(s: Raw, level: i32, name: i32) -> i32 {
        let mut v = 0i32;
        let mut n = 4i32;
        // SAFETY: a 4-byte option value.
        unsafe { getsockopt(s, level, name, &mut v as *mut i32 as *mut u8, &mut n) };
        v
    }
    pub fn name(s: Raw, peer: bool, buf: &mut [u8; 128], len: &mut i32) -> NetResult<()> {
        // SAFETY: buf has room for any sockaddr.
        st(unsafe { if peer { getpeername(s, buf.as_mut_ptr(), len) } else { getsockname(s, buf.as_mut_ptr(), len) } })
    }
    pub fn shut(s: Raw, how: i32) -> NetResult<()> {
        // SAFETY: plain call.
        st(unsafe { shutdown(s, how) })
    }
    pub fn poll(fds: &mut [PollFd], ms: i32) -> NetResult<usize> {
        // SAFETY: fds is a valid array.
        cnt(unsafe { WSAPoll(fds.as_mut_ptr(), fds.len() as u32, ms) })
    }

    /// the system resolver: the addresses of a host name
    pub fn resolve_system(host: &str) -> NetResult<alloc::vec::Vec<super::IpAddr>> {
        startup();
        let w: alloc::vec::Vec<u16> = host.encode_utf16().chain(core::iter::once(0)).collect();
        let hints = AddrInfoW { flags: 0, family: 0, socktype: 1, protocol: 6, addrlen: 0, canonname: core::ptr::null_mut(), addr: core::ptr::null(), next: core::ptr::null_mut() };
        let mut res: *mut AddrInfoW = core::ptr::null_mut();
        // SAFETY: valid strings and hints; the result list is freed below.
        let r = unsafe { GetAddrInfoW(w.as_ptr(), core::ptr::null(), &hints, &mut res) };
        if r != 0 {
            return Err(NetError::new(NetErrorKind::NotFound, r));
        }
        let mut out = alloc::vec::Vec::new();
        let mut p = res;
        while !p.is_null() {
            // SAFETY: p walks the list GetAddrInfoW returned.
            let ai = unsafe { &*p };
            if !ai.addr.is_null() && ai.addrlen >= 8 {
                // SAFETY: ai.addr points to ai.addrlen bytes.
                let raw = unsafe { core::slice::from_raw_parts(ai.addr, ai.addrlen) };
                if let Some(a) = super::decode_sockaddr(raw) {
                    if !out.contains(&a.ip) {
                        out.push(a.ip);
                    }
                }
            }
            p = ai.next;
        }
        // SAFETY: the list from GetAddrInfoW, freed once.
        unsafe { FreeAddrInfoW(res) };
        Ok(out)
    }
}

#[cfg(target_os = "linux")]
mod sys {
    use super::{NetError, NetErrorKind, NetResult};
    use rt::os::{syscall3, syscall6};

    pub type Raw = usize;
    pub const AF_INET: i32 = 2;
    pub const AF_INET6: i32 = 10;
    pub const SOL_SOCKET: i32 = 1;
    pub const SO_REUSEADDR: i32 = 2;
    pub const SO_ERROR: i32 = 4;
    pub const SO_KEEPALIVE: i32 = 9;
    pub const IPPROTO_IPV6: i32 = 41;
    pub const IPV6_V6ONLY: i32 = 26;
    pub const POLLIN: i16 = 1;
    pub const POLLOUT: i16 = 4;
    pub const SHUT_WR: i32 = 1;
    const SOCK_CLOEXEC: usize = 0x80000;
    const MSG_NOSIGNAL: usize = 0x4000;

    const SYS_POLL: usize = 7;
    const SYS_CLOSE: usize = 3;
    const SYS_SOCKET: usize = 41;
    const SYS_CONNECT: usize = 42;
    const SYS_SENDTO: usize = 44;
    const SYS_RECVFROM: usize = 45;
    const SYS_SHUTDOWN: usize = 48;
    const SYS_BIND: usize = 49;
    const SYS_LISTEN: usize = 50;
    const SYS_GETSOCKNAME: usize = 51;
    const SYS_GETPEERNAME: usize = 52;
    const SYS_SETSOCKOPT: usize = 54;
    const SYS_GETSOCKOPT: usize = 55;
    const SYS_FCNTL: usize = 72;
    const SYS_ACCEPT4: usize = 288;
    const EINTR: isize = 4;

    #[repr(C)]
    pub struct PollFd {
        pub fd: i32,
        pub events: i16,
        pub revents: i16,
    }

    pub fn kind(c: i32) -> NetErrorKind {
        match c {
            110 | 11 => NetErrorKind::TimedOut,
            111 => NetErrorKind::Refused,
            104 | 103 | 32 => NetErrorKind::Reset,
            98 => NetErrorKind::AddrInUse,
            99 => NetErrorKind::AddrNotAvailable,
            101 | 113 => NetErrorKind::Unreachable,
            _ => NetErrorKind::Other,
        }
    }

    /// a syscall result: >= 0 the value, else -errno (calls interrupted by a signal repeat)
    fn call(mut f: impl FnMut() -> isize) -> NetResult<usize> {
        loop {
            let v = f();
            if v == -EINTR {
                continue;
            }
            if v < 0 {
                let c = (-v) as i32;
                return Err(NetError::new(kind(c), c));
            }
            return Ok(v as usize);
        }
    }

    pub fn would_block(e: &NetError) -> bool {
        e.code == 11 || e.code == 115
    }

    pub fn socket(af: i32, ty: i32, proto: i32) -> NetResult<Raw> {
        // SAFETY: plain socket creation.
        call(|| unsafe { syscall3(SYS_SOCKET, af as usize, ty as usize | SOCK_CLOEXEC, proto as usize) })
    }

    pub fn close(s: Raw) {
        // SAFETY: the caller owns the descriptor.
        unsafe { syscall3(SYS_CLOSE, s, 0, 0) };
    }

    pub fn set_nonblocking(s: Raw, on: bool) -> NetResult<()> {
        // SAFETY: F_GETFL on an owned descriptor.
        let fl = call(|| unsafe { syscall3(SYS_FCNTL, s, 3, 0) })?;
        let fl = if on { fl | 0x800 } else { fl & !0x800 };
        // SAFETY: F_SETFL on an owned descriptor.
        call(|| unsafe { syscall3(SYS_FCNTL, s, 4, fl) }).map(|_| ())
    }

    pub fn bind_to(s: Raw, a: &[u8]) -> NetResult<()> {
        // SAFETY: a is a complete sockaddr.
        call(|| unsafe { syscall3(SYS_BIND, s, a.as_ptr() as usize, a.len()) }).map(|_| ())
    }
    pub fn listen_on(s: Raw, n: i32) -> NetResult<()> {
        // SAFETY: plain call.
        call(|| unsafe { syscall3(SYS_LISTEN, s, n as usize, 0) }).map(|_| ())
    }
    pub fn accept_one(s: Raw, buf: &mut [u8; 128], len: &mut i32) -> NetResult<Raw> {
        let (bp, lp) = (buf.as_mut_ptr() as usize, len as *mut i32 as usize);
        // SAFETY: buf has room for any sockaddr.
        call(|| unsafe { syscall6(SYS_ACCEPT4, s, bp, lp, SOCK_CLOEXEC, 0, 0) })
    }
    pub fn connect_to(s: Raw, a: &[u8]) -> NetResult<()> {
        // SAFETY: a is a complete sockaddr.  EINTR: the connection goes on (poll for it).
        let v = unsafe { syscall3(SYS_CONNECT, s, a.as_ptr() as usize, a.len()) };
        if v < 0 {
            let c = if v == -EINTR { 115 } else { (-v) as i32 };
            return Err(NetError::new(kind(c), c));
        }
        Ok(())
    }
    pub fn recv_into(s: Raw, b: &mut [u8]) -> NetResult<usize> {
        let (p, n) = (b.as_mut_ptr() as usize, b.len());
        // SAFETY: b is writable.
        call(|| unsafe { syscall6(SYS_RECVFROM, s, p, n, 0, 0, 0) })
    }
    pub fn send_from(s: Raw, b: &[u8]) -> NetResult<usize> {
        // SAFETY: b is readable; MSG_NOSIGNAL: an error instead of SIGPIPE.
        call(|| unsafe { syscall6(SYS_SENDTO, s, b.as_ptr() as usize, b.len(), MSG_NOSIGNAL, 0, 0) })
    }
    pub fn recv_from(s: Raw, b: &mut [u8], from: &mut [u8; 128], len: &mut i32) -> NetResult<usize> {
        let (p, n, f, l) = (b.as_mut_ptr() as usize, b.len(), from.as_mut_ptr() as usize, len as *mut i32 as usize);
        // SAFETY: valid buffers.
        call(|| unsafe { syscall6(SYS_RECVFROM, s, p, n, 0, f, l) })
    }
    pub fn send_to(s: Raw, b: &[u8], to: &[u8]) -> NetResult<usize> {
        // SAFETY: valid buffers.
        call(|| unsafe { syscall6(SYS_SENDTO, s, b.as_ptr() as usize, b.len(), MSG_NOSIGNAL, to.as_ptr() as usize, to.len()) })
    }
    pub fn set_opt(s: Raw, level: i32, name: i32, v: i32) -> NetResult<()> {
        let b = v.to_ne_bytes();
        // SAFETY: a 4-byte option value.
        call(|| unsafe { syscall6(SYS_SETSOCKOPT, s, level as usize, name as usize, b.as_ptr() as usize, 4, 0) }).map(|_| ())
    }
    pub fn get_opt(s: Raw, level: i32, name: i32) -> i32 {
        let mut v = 0i32;
        let mut n = 4u32;
        // SAFETY: a 4-byte option value.
        unsafe { syscall6(SYS_GETSOCKOPT, s, level as usize, name as usize, &mut v as *mut i32 as usize, &mut n as *mut u32 as usize, 0) };
        v
    }
    pub fn name(s: Raw, peer: bool, buf: &mut [u8; 128], len: &mut i32) -> NetResult<()> {
        let (bp, lp) = (buf.as_mut_ptr() as usize, len as *mut i32 as usize);
        let nr = if peer { SYS_GETPEERNAME } else { SYS_GETSOCKNAME };
        // SAFETY: buf has room for any sockaddr.
        call(|| unsafe { syscall3(nr, s, bp, lp) }).map(|_| ())
    }
    pub fn shut(s: Raw, how: i32) -> NetResult<()> {
        // SAFETY: plain call.
        call(|| unsafe { syscall3(SYS_SHUTDOWN, s, how as usize, 0) }).map(|_| ())
    }
    pub fn poll(fds: &mut [PollFd], ms: i32) -> NetResult<usize> {
        let (p, n) = (fds.as_mut_ptr() as usize, fds.len());
        // SAFETY: fds is a valid array.
        call(|| unsafe { syscall3(SYS_POLL, p, n, ms as isize as usize) })
    }
}

fn encode_sockaddr(a: &SocketAddr) -> Vec<u8> {
    match a.ip {
        IpAddr::V4(ip) => {
            let mut b = alloc::vec![0u8; 16];
            b[..2].copy_from_slice(&(sys::AF_INET as u16).to_ne_bytes());
            b[2..4].copy_from_slice(&a.port.to_be_bytes());
            b[4..8].copy_from_slice(&ip);
            b
        }
        IpAddr::V6(ip) => {
            let mut b = alloc::vec![0u8; 28];
            b[..2].copy_from_slice(&(sys::AF_INET6 as u16).to_ne_bytes());
            b[2..4].copy_from_slice(&a.port.to_be_bytes());
            b[8..24].copy_from_slice(&ip);
            b
        }
    }
}

fn decode_sockaddr(b: &[u8]) -> Option<SocketAddr> {
    if b.len() < 8 {
        return None;
    }
    let fam = u16::from_ne_bytes([b[0], b[1]]) as i32;
    let port = u16::from_be_bytes([b[2], b[3]]);
    if fam == sys::AF_INET {
        Some(SocketAddr::new(IpAddr::V4([b[4], b[5], b[6], b[7]]), port))
    } else if fam == sys::AF_INET6 && b.len() >= 24 {
        let mut ip = [0u8; 16];
        ip.copy_from_slice(&b[8..24]);
        Some(SocketAddr::new(IpAddr::V6(ip), port))
    } else {
        None
    }
}

fn family(a: &SocketAddr) -> i32 {
    if matches!(a.ip, IpAddr::V4(_)) { sys::AF_INET } else { sys::AF_INET6 }
}

/// an owned socket
struct Sock(sys::Raw);

impl Drop for Sock {
    fn drop(&mut self) {
        sys::close(self.0);
    }
}

impl Sock {
    /// wait until readable (or closed) -> false on timeout; ms None: no wait at all
    fn wait(&self, events: i16, ms: Option<u32>) -> NetResult<bool> {
        let Some(ms) = ms else { return Ok(true) };
        let mut fds = [sys::PollFd { fd: self.0 as _, events, revents: 0 }];
        Ok(sys::poll(&mut fds, ms.min(i32::MAX as u32) as i32)? > 0)
    }

    fn name(&self, peer: bool) -> NetResult<SocketAddr> {
        let mut buf = [0u8; 128];
        let mut len = 128i32;
        sys::name(self.0, peer, &mut buf, &mut len)?;
        decode_sockaddr(&buf[..len.max(0) as usize]).ok_or(NetError::new(NetErrorKind::Other, 0))
    }
}

fn ms_opt(v: u32) -> Option<u32> {
    if v == 0 { None } else { Some(v) }
}

// ------------------------------------------------------------------ TCP

pub struct TcpListener {
    s: Sock,
}

impl TcpListener {
    /// listen on an address (port 0: any free port, see local_addr)
    pub fn bind(addr: &SocketAddr) -> NetResult<TcpListener> {
        if !crate::policy::net_listen_allowed(addr) {
            return Err(denied());
        }
        let s = Sock(sys::socket(family(addr), 1, 6)?);
        let _ = sys::set_opt(s.0, sys::SOL_SOCKET, sys::SO_REUSEADDR, 1);
        if matches!(addr.ip, IpAddr::V6(_)) {
            let _ = sys::set_opt(s.0, sys::IPPROTO_IPV6, sys::IPV6_V6ONLY, 1);
        }
        sys::bind_to(s.0, &encode_sockaddr(addr))?;
        sys::listen_on(s.0, 512)?;
        Ok(TcpListener { s })
    }

    pub fn local_addr(&self) -> NetResult<SocketAddr> {
        self.s.name(false)
    }

    /// the next connection (blocks)
    pub fn accept(&self) -> NetResult<(TcpStream, SocketAddr)> {
        let mut buf = [0u8; 128];
        let mut len = 128i32;
        let c = sys::accept_one(self.s.0, &mut buf, &mut len)?;
        let peer = decode_sockaddr(&buf[..len.max(0) as usize]).unwrap_or(SocketAddr::new(IpAddr::UNSPECIFIED_V4, 0));
        Ok((TcpStream::from_raw(c), peer))
    }

    /// the next connection within `ms` milliseconds (None: none came)
    pub fn accept_timeout(&self, ms: u32) -> NetResult<Option<(TcpStream, SocketAddr)>> {
        if !self.s.wait(sys::POLLIN, Some(ms))? {
            return Ok(None);
        }
        self.accept().map(Some)
    }
}

pub struct TcpStream {
    s: Sock,
    read_ms: AtomicU32,
    write_ms: AtomicU32,
}

impl TcpStream {
    fn from_raw(s: sys::Raw) -> TcpStream {
        TcpStream { s: Sock(s), read_ms: AtomicU32::new(0), write_ms: AtomicU32::new(0) }
    }

    /// connect within `timeout_ms` (0: the system's own timeout)
    pub fn connect(addr: &SocketAddr, timeout_ms: u32) -> NetResult<TcpStream> {
        if !crate::policy::net_connect_allowed(addr) {
            return Err(denied());
        }
        let s = Sock(sys::socket(family(addr), 1, 6)?);
        let sa = encode_sockaddr(addr);
        if timeout_ms == 0 {
            sys::connect_to(s.0, &sa)?;
        } else {
            sys::set_nonblocking(s.0, true)?;
            if let Err(e) = sys::connect_to(s.0, &sa) {
                if !sys::would_block(&e) {
                    return Err(e);
                }
                if !s.wait(sys::POLLOUT, Some(timeout_ms))? {
                    return Err(timed_out());
                }
                let code = sys::get_opt(s.0, sys::SOL_SOCKET, sys::SO_ERROR);
                if code != 0 {
                    return Err(NetError::new(sys::kind(code), code));
                }
            }
            sys::set_nonblocking(s.0, false)?;
        }
        let raw = s.0;
        core::mem::forget(s);
        Ok(TcpStream::from_raw(raw))
    }

    /// connect to the first reachable of several addresses
    pub fn connect_any(addrs: &[SocketAddr], timeout_ms: u32) -> NetResult<TcpStream> {
        let mut last = NetError::new(NetErrorKind::NotFound, 0);
        for a in addrs {
            match TcpStream::connect(a, timeout_ms) {
                Ok(s) => return Ok(s),
                Err(e) => last = e,
            }
        }
        Err(last)
    }

    /// read timeout in ms for every read (None: block)
    pub fn set_read_timeout(&self, ms: Option<u32>) {
        self.read_ms.store(ms.unwrap_or(0), Ordering::Relaxed);
    }

    pub fn set_write_timeout(&self, ms: Option<u32>) {
        self.write_ms.store(ms.unwrap_or(0), Ordering::Relaxed);
    }

    pub fn set_nodelay(&self, on: bool) -> NetResult<()> {
        sys::set_opt(self.s.0, 6, 1, on as i32)
    }

    pub fn set_keepalive(&self, on: bool) -> NetResult<()> {
        sys::set_opt(self.s.0, sys::SOL_SOCKET, sys::SO_KEEPALIVE, on as i32)
    }

    /// up to buf.len() bytes; Ok(0) = the peer closed its side
    pub fn read(&self, buf: &mut [u8]) -> NetResult<usize> {
        if !self.s.wait(sys::POLLIN, ms_opt(self.read_ms.load(Ordering::Relaxed)))? {
            return Err(timed_out());
        }
        sys::recv_into(self.s.0, buf)
    }

    /// exactly buf.len() bytes (NetErrorKind::Eof if the peer closes first)
    pub fn read_exact(&self, buf: &mut [u8]) -> NetResult<()> {
        let mut done = 0;
        while done < buf.len() {
            let n = self.read(&mut buf[done..])?;
            if n == 0 {
                return Err(NetError::new(NetErrorKind::Eof, 0));
            }
            done += n;
        }
        Ok(())
    }

    pub fn write(&self, buf: &[u8]) -> NetResult<usize> {
        if !self.s.wait(sys::POLLOUT, ms_opt(self.write_ms.load(Ordering::Relaxed)))? {
            return Err(timed_out());
        }
        sys::send_from(self.s.0, buf)
    }

    pub fn write_all(&self, mut buf: &[u8]) -> NetResult<()> {
        while !buf.is_empty() {
            let n = self.write(buf)?;
            if n == 0 {
                return Err(NetError::new(NetErrorKind::Reset, 0));
            }
            buf = &buf[n..];
        }
        Ok(())
    }

    /// whether data (or the end) is waiting, within `ms`
    pub fn wait_readable(&self, ms: u32) -> NetResult<bool> {
        self.s.wait(sys::POLLIN, Some(ms))
    }

    /// no more writes from this side (the peer reads the end)
    pub fn shutdown_write(&self) -> NetResult<()> {
        sys::shut(self.s.0, sys::SHUT_WR)
    }

    pub fn peer_addr(&self) -> NetResult<SocketAddr> {
        self.s.name(true)
    }

    pub fn local_addr(&self) -> NetResult<SocketAddr> {
        self.s.name(false)
    }
}

// ------------------------------------------------------------------ UDP

pub struct UdpSocket {
    s: Sock,
}

impl UdpSocket {
    /// a socket on addr; a fixed port needs net.listen, port 0 (a client's) does not
    pub fn bind(addr: &SocketAddr) -> NetResult<UdpSocket> {
        if addr.port != 0 && !crate::policy::net_listen_allowed(addr) {
            return Err(denied());
        }
        let s = Sock(sys::socket(family(addr), 2, 17)?);
        sys::bind_to(s.0, &encode_sockaddr(addr))?;
        Ok(UdpSocket { s })
    }

    pub fn send_to(&self, buf: &[u8], to: &SocketAddr) -> NetResult<usize> {
        if !crate::policy::net_connect_allowed(to) {
            return Err(denied());
        }
        sys::send_to(self.s.0, buf, &encode_sockaddr(to))
    }

    /// one datagram within `timeout_ms` (0: block)
    pub fn recv_from(&self, buf: &mut [u8], timeout_ms: u32) -> NetResult<(usize, SocketAddr)> {
        if !self.s.wait(sys::POLLIN, ms_opt(timeout_ms))? {
            return Err(timed_out());
        }
        let mut from = [0u8; 128];
        let mut len = 128i32;
        let n = sys::recv_from(self.s.0, buf, &mut from, &mut len)?;
        let a = decode_sockaddr(&from[..len.max(0) as usize]).unwrap_or(SocketAddr::new(IpAddr::UNSPECIFIED_V4, 0));
        Ok((n, a))
    }

    pub fn local_addr(&self) -> NetResult<SocketAddr> {
        self.s.name(false)
    }
}

// ------------------------------------------------------------------ Unix domain sockets

/// AF_UNIX on Linux and Windows
const AF_UNIX: i32 = 1;

/// a connected Unix domain stream socket (the stream calls of TcpStream)
pub type UnixStream = TcpStream;

/// sockaddr_un of a socket path (sun_path holds 108 bytes on both systems)
fn unix_addr(path: &str) -> NetResult<Vec<u8>> {
    let p = path.as_bytes();
    if p.is_empty() || p.len() >= 108 || p.contains(&0) {
        return Err(NetError::new(NetErrorKind::Other, 0));
    }
    let mut b = alloc::vec![0u8; 2 + 108];
    b[..2].copy_from_slice(&(AF_UNIX as u16).to_ne_bytes());
    b[2..2 + p.len()].copy_from_slice(p);
    Ok(b)
}

pub struct UnixListener {
    s: Sock,
    path: String,
}

impl UnixListener {
    /// listen on a socket file; one left over at `path` is removed first
    pub fn bind(path: &str) -> NetResult<UnixListener> {
        if !crate::policy::fs_allowed(path, crate::policy::FS_WRITE) {
            return Err(denied());
        }
        let a = unix_addr(path)?;
        crate::delete_raw(path);
        let s = Sock(sys::socket(AF_UNIX, 1, 0)?);
        sys::bind_to(s.0, &a)?;
        sys::listen_on(s.0, 64)?;
        Ok(UnixListener { s, path: String::from(path) })
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    pub fn accept(&self) -> NetResult<UnixStream> {
        let mut buf = [0u8; 128];
        let mut len = 128i32;
        let c = sys::accept_one(self.s.0, &mut buf, &mut len)?;
        Ok(TcpStream::from_raw(c))
    }

    /// the next connection within `ms` milliseconds (None: none came)
    pub fn accept_timeout(&self, ms: u32) -> NetResult<Option<UnixStream>> {
        if !self.s.wait(sys::POLLIN, Some(ms))? {
            return Ok(None);
        }
        self.accept().map(Some)
    }
}

impl Drop for UnixListener {
    fn drop(&mut self) {
        crate::delete_raw(&self.path);
    }
}

/// connect to a socket file within `timeout_ms` (0: block)
pub fn connect_unix(path: &str, timeout_ms: u32) -> NetResult<UnixStream> {
    if !crate::policy::fs_allowed(path, crate::policy::FS_WRITE) {
        return Err(denied());
    }
    let a = unix_addr(path)?;
    let s = Sock(sys::socket(AF_UNIX, 1, 0)?);
    if timeout_ms == 0 {
        sys::connect_to(s.0, &a)?;
    } else {
        sys::set_nonblocking(s.0, true)?;
        if let Err(e) = sys::connect_to(s.0, &a) {
            if !sys::would_block(&e) {
                return Err(e);
            }
            if !s.wait(sys::POLLOUT, Some(timeout_ms))? {
                return Err(timed_out());
            }
            let code = sys::get_opt(s.0, sys::SOL_SOCKET, sys::SO_ERROR);
            if code != 0 {
                return Err(NetError::new(sys::kind(code), code));
            }
        }
        sys::set_nonblocking(s.0, false)?;
    }
    let raw = s.0;
    core::mem::forget(s);
    Ok(TcpStream::from_raw(raw))
}

// ------------------------------------------------------------------ names

/// the addresses of a host: numeric forms and "localhost" everywhere; on Windows other names
/// through the system resolver (Linux: netio::dns)
pub fn resolve(host: &str, port: u16) -> NetResult<Vec<SocketAddr>> {
    let h = host.trim_start_matches('[').trim_end_matches(']');
    if let Some(ip) = IpAddr::parse(h) {
        return Ok(alloc::vec![SocketAddr::new(ip, port)]);
    }
    if h.eq_ignore_ascii_case("localhost") {
        return Ok(alloc::vec![SocketAddr::new(IpAddr::LOCALHOST_V4, port), SocketAddr::new(IpAddr::LOCALHOST_V6, port)]);
    }
    #[cfg(windows)]
    {
        if !crate::policy::net_name_allowed(h) {
            return Err(denied());
        }
        let ips = sys::resolve_system(h)?;
        if ips.is_empty() {
            return Err(NetError::new(NetErrorKind::NotFound, 0));
        }
        let addrs: Vec<SocketAddr> = ips.into_iter().map(|ip| SocketAddr::new(ip, port)).collect();
        crate::policy::net_resolved(h, &addrs);
        Ok(addrs)
    }
    #[cfg(target_os = "linux")]
    {
        Err(NetError::new(NetErrorKind::NotFound, 0))
    }
}

/// the host name of this computer
pub fn hostname() -> String {
    #[cfg(windows)]
    {
        crate::env("COMPUTERNAME").map(|s| s.to_lowercase()).unwrap_or_else(|| String::from("localhost"))
    }
    #[cfg(target_os = "linux")]
    {
        crate::read_file_raw("/proc/sys/kernel/hostname")
            .map(|b| String::from(String::from_utf8_lossy(&b).trim()))
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| String::from("localhost"))
    }
}

/// "host:port" with brackets around IPv6 literals
pub fn host_port(host: &str, port: u16) -> String {
    if host.contains(':') && !host.starts_with('[') { format!("[{}]:{}", host, port) } else { format!("{}:{}", host, port) }
}
