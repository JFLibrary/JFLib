// net -- TCP, UDP and Unix domain sockets (IPv4 and IPv6) with timeouts
//
//   IpAddr / SocketAddr: parse ("10.0.0.5:9443", "[::1]:80", "[fe80::1%eth0]:53"), Display,
//             loopback / unspecified; SocketAddr::scope: the IPv6 zone (sin6_scope_id, the
//             interface index of a link-local address; 0 none)
//   ip_scope_parse("fe80::1%eth0") -> (address, interface index): a zone by name (Linux:
//             /sys/class/net/NAME/ifindex, else ioctl SIOCGIFINDEX; Windows: if_nametoindex) or
//             by number ("%2")
//   local_addrs() -> this computer's addresses (port 0, link-local ones with their scope):
//             Windows GetAdaptersAddresses; Linux SIOCGIFCONF (IPv4), /proc/net/if_inet6 (IPv6)
//   dns_servers() -> (Windows) the DNS servers of the adapters that are up, with their scope
//             (the well-known site-local fec0:0:0:ffff::1..3 Windows lists without IPv6 DNS
//             are left out); Linux: empty (netio::dns reads /etc/resolv.conf)
//   TcpListener::bind(&addr) / accept() / accept_timeout(ms) / local_addr()
//   TcpStream::connect(&addr, timeout_ms) / read / write / write_all / read_exact,
//             set_read_timeout / set_write_timeout (ms, None = block), set_nodelay,
//             shutdown_write, peer_addr / local_addr, wait_readable(ms)
//   UdpSocket::bind / send_to / recv_from(buf, timeout_ms)
//   UnixListener::bind(path) / accept() / accept_timeout(ms); connect_unix(path, timeout_ms)
//             (bind removes only a socket file nobody listens on; the drop only its own)
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
                if w.is_empty() || w.len() > 4 || !w.bytes().all(|b| b.is_ascii_hexdigit()) {
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
    /// IPv6: the zone of a link-local (or other scoped) address, the interface index the
    /// socket address carries as sin6_scope_id (0: none)
    pub scope: u32,
}

impl SocketAddr {
    pub const fn new(ip: IpAddr, port: u16) -> SocketAddr {
        SocketAddr { ip, port, scope: 0 }
    }

    /// an IPv6 address with its zone (the interface index; ip_scope_parse)
    pub const fn with_scope(ip: IpAddr, port: u16, scope: u32) -> SocketAddr {
        SocketAddr { ip, port, scope }
    }

    /// "1.2.3.4:80", "[::1]:80", "[fe80::1%eth0]:53" (a zone by name or number)
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
        if port.is_empty() || !port.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let (ip, scope) = ip_scope_parse(host)?;
        Some(SocketAddr { ip, port: port.parse().ok()?, scope })
    }
}

impl fmt::Display for SocketAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.ip {
            IpAddr::V4(_) => write!(f, "{}:{}", self.ip, self.port),
            IpAddr::V6(_) if self.scope != 0 => write!(f, "[{}%{}]:{}", self.ip, self.scope, self.port),
            IpAddr::V6(_) => write!(f, "[{}]:{}", self.ip, self.port),
        }
    }
}

/// an IP address with an optional IPv6 zone ("fe80::1%eth0", "fe80::1%2") -> the address and
/// the zone's interface index (0: none).  None: not an address, or a zone that names no
/// interface of this computer (a zone on an IPv4 address too)
pub fn ip_scope_parse(s: &str) -> Option<(IpAddr, u32)> {
    let Some((a, zone)) = s.split_once('%') else { return Some((IpAddr::parse(s)?, 0)) };
    let ip = IpAddr::parse(a)?;
    if !matches!(ip, IpAddr::V6(_)) || zone.is_empty() {
        return None;
    }
    let scope = if zone.bytes().all(|b| b.is_ascii_digit()) { zone.parse::<u32>().ok()? } else { if_index(zone)? };
    Some((ip, scope))
}

/// the index of a network interface by its name (Linux: "eth0"; Windows: the NDIS name, e.g.
/// "ethernet_32769") -> None: there is none
pub fn if_index(name: &str) -> Option<u32> {
    if name.is_empty() || name.len() > 255 || name.bytes().any(|b| b == 0 || b == b'/') || name == "." || name == ".." {
        return None;
    }
    sys::if_index(name).filter(|i| *i != 0)
}

/// this computer's addresses (port 0; link-local IPv6 addresses with their scope), see the top
/// of this file
pub fn local_addrs() -> Vec<SocketAddr> {
    let mut v = sys::local_addrs();
    v.dedup();
    v
}

/// (Windows) the DNS servers of the network adapters that are up (port 53, link-local ones
/// with their scope), in the system's adapter order; Linux: empty (netio::dns reads
/// /etc/resolv.conf)
pub fn dns_servers() -> Vec<SocketAddr> {
    let mut out: Vec<SocketAddr> = Vec::new();
    for a in sys::dns_servers() {
        // Windows' stand-ins without IPv6 DNS: fec0:0:0:ffff::1, ::2, ::3 (site-local, deprecated)
        let standin = matches!(a.ip, IpAddr::V6(b) if b[..14] == [0xfe, 0xc0, 0, 0, 0, 0, 0xff, 0xff, 0, 0, 0, 0, 0, 0] && (1..=3).contains(&b[15]) && b[14] == 0);
        if !standin && !out.contains(&a) {
            out.push(a);
        }
    }
    out
}

// ------------------------------------------------------------------ errors

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
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

    #[link(name = "iphlpapi", kind = "raw-dylib")]
    unsafe extern "system" {
        fn GetAdaptersAddresses(family: u32, flags: u32, reserved: usize, buf: *mut u8, size: *mut u32) -> u32;
        fn if_nametoindex(name: *const u8) -> u32;
    }

    static STARTED: AtomicU8 = AtomicU8::new(0);

    /// if_nametoindex (iphlpapi: the NDIS interface name)
    pub fn if_index(name: &str) -> Option<u32> {
        let z: alloc::vec::Vec<u8> = name.bytes().chain(core::iter::once(0)).collect();
        // SAFETY: a NUL-terminated name.
        let i = unsafe { if_nametoindex(z.as_ptr()) };
        (i != 0).then_some(i)
    }

    // the adapters of GetAdaptersAddresses (flags: GAA_FLAG_*) -> f(record) for each one that
    // is up, the record's bytes valid during the call
    fn adapters(flags: u32, f: &mut dyn FnMut(*const u8)) {
        let mut size = 16u32 << 10;
        for _ in 0..4 {
            let mut buf = alloc::vec![0u64; (size as usize).div_ceil(8)];
            // SAFETY: a buffer of `size` bytes (8-aligned); size is updated on overflow.
            let r = unsafe { GetAdaptersAddresses(0, flags, 0, buf.as_mut_ptr() as *mut u8, &mut size) };
            if r == 111 {
                continue; // ERROR_BUFFER_OVERFLOW: size holds what it needs
            }
            if r != 0 {
                return;
            }
            const P: usize = core::mem::size_of::<usize>();
            let mut a = buf.as_ptr() as *const u8;
            while !a.is_null() {
                // SAFETY: IP_ADAPTER_ADDRESSES records inside buf, linked by Next (after the
                // 8-byte Length / IfIndex head); OperStatus after the fixed fields.
                let (up, next) = unsafe { ((a.add(32 + 9 * P) as *const u32).read_unaligned() == 1, (a.add(8) as *const *const u8).read_unaligned()) };
                if up {
                    f(a);
                }
                a = next;
            }
            return;
        }
    }

    // the socket addresses of one list of IP_ADAPTER_*_ADDRESS records (head 8 bytes, Next,
    // SOCKET_ADDRESS { lpSockaddr, iSockaddrLength })
    fn sockaddrs(mut e: *const u8, out: &mut alloc::vec::Vec<super::SocketAddr>, port: u16) {
        const P: usize = core::mem::size_of::<usize>();
        for _ in 0..64 {
            if e.is_null() {
                return;
            }
            // SAFETY: a record of the list GetAdaptersAddresses filled (see adapters).
            let (sa, len, next) = unsafe {
                ((e.add(8 + P) as *const *const u8).read_unaligned(), (e.add(8 + 2 * P) as *const i32).read_unaligned(), (e.add(8) as *const *const u8).read_unaligned())
            };
            if !sa.is_null() && (8..=128).contains(&len) {
                // SAFETY: lpSockaddr points to iSockaddrLength bytes.
                let raw = unsafe { core::slice::from_raw_parts(sa, len as usize) };
                if let Some(mut a) = super::decode_sockaddr(raw) {
                    a.port = port;
                    out.push(a);
                }
            }
            e = next;
        }
    }

    /// the unicast addresses of the adapters that are up
    pub fn local_addrs() -> alloc::vec::Vec<super::SocketAddr> {
        const P: usize = core::mem::size_of::<usize>();
        let mut out = alloc::vec::Vec::new();
        // GAA_FLAG_SKIP_ANYCAST | SKIP_MULTICAST | SKIP_DNS_SERVER | SKIP_FRIENDLY_NAME
        adapters(0x2 | 0x4 | 0x8 | 0x20, &mut |a| {
            // SAFETY: FirstUnicastAddress of the record.
            let first = unsafe { (a.add(8 + 2 * P) as *const *const u8).read_unaligned() };
            sockaddrs(first, &mut out, 0);
        });
        out
    }

    /// the DNS servers of the adapters that are up
    pub fn dns_servers() -> alloc::vec::Vec<super::SocketAddr> {
        const P: usize = core::mem::size_of::<usize>();
        let mut out = alloc::vec::Vec::new();
        // GAA_FLAG_SKIP_UNICAST | SKIP_ANYCAST | SKIP_MULTICAST | SKIP_FRIENDLY_NAME
        adapters(0x1 | 0x2 | 0x4 | 0x20, &mut |a| {
            // SAFETY: FirstDnsServerAddress of the record.
            let first = unsafe { (a.add(8 + 5 * P) as *const *const u8).read_unaligned() };
            sockaddrs(first, &mut out, 53);
        });
        out
    }

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
    use rt::os::{syscall3, syscall6, O_NONBLOCK, SYS_ACCEPT4, SYS_BIND, SYS_CLOSE, SYS_CONNECT, SYS_FCNTL, SYS_GETPEERNAME, SYS_GETSOCKNAME, SYS_GETSOCKOPT, SYS_IOCTL, SYS_LISTEN, SYS_RECVFROM, SYS_SENDTO, SYS_SETSOCKOPT, SYS_SHUTDOWN, SYS_SOCKET};

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

    const SIOCGIFCONF: usize = 0x8912;
    const SIOCGIFINDEX: usize = 0x8933;
    const EINTR: isize = 4;

    /// /sys/class/net/NAME/ifindex, else ioctl SIOCGIFINDEX (struct ifreq: 16 bytes of name,
    /// the index)
    pub fn if_index(name: &str) -> Option<u32> {
        if let Some(b) = crate::read_file_raw(&alloc::format!("/sys/class/net/{name}/ifindex")) {
            if let Ok(i) = core::str::from_utf8(&b).unwrap_or("").trim().parse::<u32>() {
                return Some(i);
            }
        }
        if name.len() >= 16 {
            return None;
        }
        let s = socket(AF_INET, 2, 0).ok()?;
        let mut req = [0u8; 40];
        req[..name.len()].copy_from_slice(name.as_bytes());
        // SAFETY: an ifreq of 40 bytes with the NUL-terminated name.
        let r = unsafe { syscall3(SYS_IOCTL, s, SIOCGIFINDEX, req.as_mut_ptr() as usize) };
        close(s);
        (r == 0).then(|| u32::from_ne_bytes([req[16], req[17], req[18], req[19]]))
    }

    /// IPv4: SIOCGIFCONF (struct ifreq of 40 bytes: the name, a sockaddr_in); IPv6:
    /// /proc/net/if_inet6 ("address index prefix scope flags name", scope 0x20 = link-local)
    pub fn local_addrs() -> alloc::vec::Vec<super::SocketAddr> {
        let mut out = alloc::vec::Vec::new();
        if let Ok(s) = socket(AF_INET, 2, 0) {
            let mut buf = alloc::vec![0u8; 40 * 64];
            let mut conf = [0u64; 2]; // struct ifconf { int len; char *buf } (padded)
            conf[0] = buf.len() as u64;
            conf[1] = buf.as_mut_ptr() as u64;
            // SAFETY: an ifconf naming a buffer of its length.
            let r = unsafe { syscall3(SYS_IOCTL, s, SIOCGIFCONF, conf.as_mut_ptr() as usize) };
            close(s);
            if r == 0 {
                let n = (conf[0] as u32 as usize).min(buf.len());
                for req in buf[..n].chunks_exact(40) {
                    if u16::from_ne_bytes([req[16], req[17]]) == AF_INET as u16 {
                        out.push(super::SocketAddr::new(super::IpAddr::V4([req[20], req[21], req[22], req[23]]), 0));
                    }
                }
            }
        }
        let text = crate::read_file_raw("/proc/net/if_inet6").unwrap_or_default();
        for line in core::str::from_utf8(&text).unwrap_or("").lines() {
            let f: alloc::vec::Vec<&str> = line.split_whitespace().collect();
            if f.len() < 6 || f[0].len() != 32 {
                continue;
            }
            let mut a = [0u8; 16];
            if (0..16).any(|i| u8::from_str_radix(&f[0][2 * i..2 * i + 2], 16).map(|v| a[i] = v).is_err()) {
                continue;
            }
            let idx = u32::from_str_radix(f[1], 16).unwrap_or(0);
            let link = u32::from_str_radix(f[3], 16).unwrap_or(0) == 0x20;
            out.push(super::SocketAddr::with_scope(super::IpAddr::V6(a), 0, if link { idx } else { 0 }));
        }
        out
    }

    /// (Windows only: Linux names its servers in /etc/resolv.conf, netio::dns)
    pub fn dns_servers() -> alloc::vec::Vec<super::SocketAddr> {
        alloc::vec::Vec::new()
    }

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
        let fl = if on { fl | O_NONBLOCK } else { fl & !O_NONBLOCK };
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
        call(|| unsafe { rt::os::poll(p, n, ms) })
    }
}

// macOS and iOS: BSD sockets through libSystem.  Their sockaddr starts with a length byte and a family
// byte (encode_sockaddr / decode_sockaddr / unix_addr); sends never raise SIGPIPE through SO_NOSIGPIPE,
// set on every socket; names go through the system resolver (getaddrinfo: VPNs, mDNS, the iOS rules).
#[cfg(target_vendor = "apple")]
mod sys {
    use super::{NetError, NetErrorKind, NetResult};
    use core::ffi::{c_char, c_int, c_uint, c_void};

    pub type Raw = i32;
    pub const AF_INET: i32 = 2;
    pub const AF_INET6: i32 = 30;
    pub const SOL_SOCKET: i32 = 0xffff;
    pub const SO_REUSEADDR: i32 = 0x0004;
    pub const SO_ERROR: i32 = 0x1007;
    pub const SO_KEEPALIVE: i32 = 0x0008;
    pub const IPPROTO_IPV6: i32 = 41;
    pub const IPV6_V6ONLY: i32 = 27;
    pub const POLLIN: i16 = 1;
    pub const POLLOUT: i16 = 4;
    pub const SHUT_WR: i32 = 1;
    const SO_NOSIGPIPE: i32 = 0x1022;
    const F_GETFL: c_int = 3;
    const F_SETFL: c_int = 4;
    const F_SETFD: c_int = 2;
    const FD_CLOEXEC: c_int = 1;
    const O_NONBLOCK: c_int = 4;
    const EINTR: i32 = 4;
    const EINPROGRESS: i32 = 36;

    #[repr(C)]
    pub struct PollFd {
        pub fd: i32,
        pub events: i16,
        pub revents: i16,
    }
    #[repr(C)]
    struct AddrInfo {
        flags: c_int,
        family: c_int,
        socktype: c_int,
        protocol: c_int,
        addrlen: u32,
        canonname: *mut c_char,
        addr: *const u8,
        next: *mut AddrInfo,
    }
    #[repr(C)]
    struct IfAddrs {
        next: *mut IfAddrs,
        name: *const c_char,
        flags: c_uint,
        addr: *const u8,
        netmask: *const u8,
        dstaddr: *const u8,
        data: *mut c_void,
    }

    #[link(name = "System")]
    unsafe extern "C" {
        #[link_name = "socket"]
        fn c_socket(domain: c_int, ty: c_int, proto: c_int) -> c_int;
        #[link_name = "close"]
        fn c_close(fd: c_int) -> c_int;
        fn fcntl(fd: c_int, cmd: c_int, ...) -> c_int;
        fn bind(fd: c_int, a: *const u8, len: u32) -> c_int;
        fn listen(fd: c_int, n: c_int) -> c_int;
        fn accept(fd: c_int, a: *mut u8, len: *mut u32) -> c_int;
        fn connect(fd: c_int, a: *const u8, len: u32) -> c_int;
        fn recv(fd: c_int, b: *mut c_void, n: usize, flags: c_int) -> isize;
        fn send(fd: c_int, b: *const c_void, n: usize, flags: c_int) -> isize;
        fn recvfrom(fd: c_int, b: *mut c_void, n: usize, flags: c_int, from: *mut u8, len: *mut u32) -> isize;
        fn sendto(fd: c_int, b: *const c_void, n: usize, flags: c_int, to: *const u8, len: u32) -> isize;
        fn setsockopt(fd: c_int, level: c_int, name: c_int, v: *const c_void, len: u32) -> c_int;
        fn getsockopt(fd: c_int, level: c_int, name: c_int, v: *mut c_void, len: *mut u32) -> c_int;
        fn getsockname(fd: c_int, a: *mut u8, len: *mut u32) -> c_int;
        fn getpeername(fd: c_int, a: *mut u8, len: *mut u32) -> c_int;
        fn shutdown(fd: c_int, how: c_int) -> c_int;
        #[link_name = "poll"]
        fn c_poll(fds: *mut PollFd, n: c_uint, ms: c_int) -> c_int;
        fn if_nametoindex(name: *const c_char) -> c_uint;
        fn getifaddrs(out: *mut *mut IfAddrs) -> c_int;
        fn freeifaddrs(list: *mut IfAddrs);
        fn getaddrinfo(host: *const c_char, service: *const c_char, hints: *const AddrInfo, res: *mut *mut AddrInfo) -> c_int;
        fn freeaddrinfo(res: *mut AddrInfo);
        fn __error() -> *mut c_int;
    }

    fn errno() -> i32 {
        // SAFETY: the calling thread's errno slot, always valid.
        unsafe { *__error() }
    }

    /// if_nametoindex ("en0")
    pub fn if_index(name: &str) -> Option<u32> {
        let mut z = name.as_bytes().to_vec();
        z.push(0);
        // SAFETY: a NUL-terminated name.
        let i = unsafe { if_nametoindex(z.as_ptr().cast()) };
        (i != 0).then_some(i)
    }

    /// getifaddrs: every interface's IPv4 and IPv6 addresses (link-local ones with their scope)
    pub fn local_addrs() -> alloc::vec::Vec<super::SocketAddr> {
        let mut out = alloc::vec::Vec::new();
        let mut list: *mut IfAddrs = core::ptr::null_mut();
        // SAFETY: getifaddrs fills list, freed below.
        if unsafe { getifaddrs(&mut list) } != 0 {
            return out;
        }
        let mut p = list;
        while !p.is_null() {
            // SAFETY: p walks the list getifaddrs returned; an address is at least its length byte long.
            let (addr, next) = unsafe { ((*p).addr, (*p).next) };
            if !addr.is_null() {
                // SAFETY: a sockaddr whose first byte is its length.
                let raw = unsafe { core::slice::from_raw_parts(addr, (*addr).max(2) as usize) };
                if let Some(a) = super::decode_sockaddr(raw) {
                    out.push(super::SocketAddr::with_scope(a.ip, 0, a.scope));
                }
            }
            p = next;
        }
        // SAFETY: the list from getifaddrs, freed once.
        unsafe { freeifaddrs(list) };
        out
    }

    /// (Windows only: Apple resolves names through getaddrinfo, resolve_system)
    pub fn dns_servers() -> alloc::vec::Vec<super::SocketAddr> {
        alloc::vec::Vec::new()
    }

    pub fn kind(c: i32) -> NetErrorKind {
        match c {
            60 | 35 => NetErrorKind::TimedOut,
            61 => NetErrorKind::Refused,
            54 | 53 | 32 => NetErrorKind::Reset,
            48 => NetErrorKind::AddrInUse,
            49 => NetErrorKind::AddrNotAvailable,
            51 | 65 => NetErrorKind::Unreachable,
            _ => NetErrorKind::Other,
        }
    }

    /// a libSystem result: >= 0 the value, else errno (calls interrupted by a signal repeat)
    fn call(mut f: impl FnMut() -> isize) -> NetResult<usize> {
        loop {
            let v = f();
            if v >= 0 {
                return Ok(v as usize);
            }
            let c = errno();
            if c != EINTR {
                return Err(NetError::new(kind(c), c));
            }
        }
    }

    pub fn would_block(e: &NetError) -> bool {
        e.code == 35 || e.code == EINPROGRESS
    }

    /// a new socket or an accepted one: not inherited, no SIGPIPE
    fn own(fd: c_int) -> Raw {
        // SAFETY: an owned descriptor; FD_CLOEXEC and SO_NOSIGPIPE take an int.
        unsafe {
            fcntl(fd, F_SETFD, FD_CLOEXEC);
            let one: c_int = 1;
            setsockopt(fd, SOL_SOCKET, SO_NOSIGPIPE, (&one as *const c_int).cast(), 4);
        }
        fd
    }

    pub fn socket(af: i32, ty: i32, proto: i32) -> NetResult<Raw> {
        // SAFETY: plain socket creation.
        call(|| unsafe { c_socket(af, ty, proto) } as isize).map(|fd| own(fd as c_int))
    }

    pub fn close(s: Raw) {
        // SAFETY: the caller owns the descriptor.
        unsafe { c_close(s) };
    }

    pub fn set_nonblocking(s: Raw, on: bool) -> NetResult<()> {
        // SAFETY: F_GETFL / F_SETFL on an owned descriptor.
        let fl = call(|| unsafe { fcntl(s, F_GETFL) } as isize)? as c_int;
        let fl = if on { fl | O_NONBLOCK } else { fl & !O_NONBLOCK };
        call(|| unsafe { fcntl(s, F_SETFL, fl) } as isize).map(|_| ())
    }

    pub fn bind_to(s: Raw, a: &[u8]) -> NetResult<()> {
        // SAFETY: a is a complete sockaddr.
        call(|| unsafe { bind(s, a.as_ptr(), a.len() as u32) } as isize).map(|_| ())
    }
    pub fn listen_on(s: Raw, n: i32) -> NetResult<()> {
        // SAFETY: plain call.
        call(|| unsafe { listen(s, n) } as isize).map(|_| ())
    }
    pub fn accept_one(s: Raw, buf: &mut [u8; 128], len: &mut i32) -> NetResult<Raw> {
        let lp = len as *mut i32 as *mut u32;
        // SAFETY: buf has room for any sockaddr; len is its size.
        call(|| unsafe { accept(s, buf.as_mut_ptr(), lp) } as isize).map(|fd| own(fd as c_int))
    }
    pub fn connect_to(s: Raw, a: &[u8]) -> NetResult<()> {
        // SAFETY: a is a complete sockaddr.  EINTR: the connection goes on (poll for it).
        if unsafe { connect(s, a.as_ptr(), a.len() as u32) } < 0 {
            let c = errno();
            let c = if c == EINTR { EINPROGRESS } else { c };
            return Err(NetError::new(kind(c), c));
        }
        Ok(())
    }
    pub fn recv_into(s: Raw, b: &mut [u8]) -> NetResult<usize> {
        // SAFETY: b is writable for its length.
        call(|| unsafe { recv(s, b.as_mut_ptr().cast(), b.len(), 0) })
    }
    pub fn send_from(s: Raw, b: &[u8]) -> NetResult<usize> {
        // SAFETY: b is readable for its length.
        call(|| unsafe { send(s, b.as_ptr().cast(), b.len(), 0) })
    }
    pub fn recv_from(s: Raw, b: &mut [u8], from: &mut [u8; 128], len: &mut i32) -> NetResult<usize> {
        let lp = len as *mut i32 as *mut u32;
        // SAFETY: valid buffers.
        call(|| unsafe { recvfrom(s, b.as_mut_ptr().cast(), b.len(), 0, from.as_mut_ptr(), lp) })
    }
    pub fn send_to(s: Raw, b: &[u8], to: &[u8]) -> NetResult<usize> {
        // SAFETY: valid buffers.
        call(|| unsafe { sendto(s, b.as_ptr().cast(), b.len(), 0, to.as_ptr(), to.len() as u32) })
    }
    pub fn set_opt(s: Raw, level: i32, name: i32, v: i32) -> NetResult<()> {
        // SAFETY: a 4-byte option value.
        call(|| unsafe { setsockopt(s, level, name, (&v as *const i32).cast(), 4) } as isize).map(|_| ())
    }
    pub fn get_opt(s: Raw, level: i32, name: i32) -> i32 {
        let mut v = 0i32;
        let mut n = 4u32;
        // SAFETY: a 4-byte option value.
        unsafe { getsockopt(s, level, name, (&mut v as *mut i32).cast(), &mut n) };
        v
    }
    pub fn name(s: Raw, peer: bool, buf: &mut [u8; 128], len: &mut i32) -> NetResult<()> {
        let lp = len as *mut i32 as *mut u32;
        // SAFETY: buf has room for any sockaddr; len is its size.
        call(|| unsafe { if peer { getpeername(s, buf.as_mut_ptr(), lp) } else { getsockname(s, buf.as_mut_ptr(), lp) } } as isize)
            .map(|_| ())
    }
    pub fn shut(s: Raw, how: i32) -> NetResult<()> {
        // SAFETY: plain call.
        call(|| unsafe { shutdown(s, how) } as isize).map(|_| ())
    }
    pub fn poll(fds: &mut [PollFd], ms: i32) -> NetResult<usize> {
        // SAFETY: fds is a valid array of its length.
        call(|| unsafe { c_poll(fds.as_mut_ptr(), fds.len() as c_uint, ms) } as isize)
    }

    /// the system resolver: the addresses of a host name (getaddrinfo)
    pub fn resolve_system(host: &str) -> NetResult<alloc::vec::Vec<super::IpAddr>> {
        let mut z = host.as_bytes().to_vec();
        z.push(0);
        let hints = AddrInfo { flags: 0, family: 0, socktype: 1, protocol: 6, addrlen: 0, canonname: core::ptr::null_mut(),
                               addr: core::ptr::null(), next: core::ptr::null_mut() };
        let mut res: *mut AddrInfo = core::ptr::null_mut();
        // SAFETY: a NUL-terminated name and valid hints; the result list is freed below.
        let r = unsafe { getaddrinfo(z.as_ptr().cast(), core::ptr::null(), &hints, &mut res) };
        if r != 0 {
            return Err(NetError::new(NetErrorKind::NotFound, r));
        }
        let mut out = alloc::vec::Vec::new();
        let mut p = res;
        while !p.is_null() {
            // SAFETY: p walks the list getaddrinfo returned.
            let ai = unsafe { &*p };
            if !ai.addr.is_null() && ai.addrlen >= 8 {
                // SAFETY: ai.addr points to ai.addrlen bytes.
                let raw = unsafe { core::slice::from_raw_parts(ai.addr, ai.addrlen as usize) };
                if let Some(a) = super::decode_sockaddr(raw) {
                    if !out.contains(&a.ip) {
                        out.push(a.ip);
                    }
                }
            }
            p = ai.next;
        }
        // SAFETY: the list from getaddrinfo, freed once.
        unsafe { freeaddrinfo(res) };
        Ok(out)
    }
}

// Apple's sockaddr: sa_len, sa_family (one byte each), then the same fields as Linux and Windows
#[cfg(target_vendor = "apple")]
fn encode_sockaddr(a: &SocketAddr) -> Vec<u8> {
    let mut b = match a.ip {
        IpAddr::V4(ip) => {
            let mut b = alloc::vec![0u8; 16];
            b[4..8].copy_from_slice(&ip);
            b
        }
        IpAddr::V6(ip) => {
            let mut b = alloc::vec![0u8; 28];
            b[8..24].copy_from_slice(&ip);
            b[24..28].copy_from_slice(&a.scope.to_ne_bytes()); // sin6_scope_id
            b
        }
    };
    b[0] = b.len() as u8;
    b[1] = family(a) as u8;
    b[2..4].copy_from_slice(&a.port.to_be_bytes());
    b
}

#[cfg(target_vendor = "apple")]
fn decode_sockaddr(b: &[u8]) -> Option<SocketAddr> {
    if b.len() < 8 {
        return None;
    }
    let port = u16::from_be_bytes([b[2], b[3]]);
    match b[1] as i32 {
        sys::AF_INET => Some(SocketAddr::new(IpAddr::V4([b[4], b[5], b[6], b[7]]), port)),
        sys::AF_INET6 if b.len() >= 24 => {
            let mut ip = [0u8; 16];
            ip.copy_from_slice(&b[8..24]);
            let mut scope = if b.len() >= 28 { u32::from_ne_bytes([b[24], b[25], b[26], b[27]]) } else { 0 };
            // the kernel's link-local addresses may carry the interface in their second word (KAME)
            if ip[0] == 0xfe && ip[1] & 0xc0 == 0x80 && (ip[2] | ip[3]) != 0 {
                if scope == 0 {
                    scope = u16::from_be_bytes([ip[2], ip[3]]) as u32;
                }
                (ip[2], ip[3]) = (0, 0);
            }
            Some(SocketAddr::with_scope(IpAddr::V6(ip), port, scope))
        }
        _ => None,
    }
}

#[cfg(not(target_vendor = "apple"))]
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
            b[24..28].copy_from_slice(&a.scope.to_ne_bytes()); // sin6_scope_id
            b
        }
    }
}

#[cfg(not(target_vendor = "apple"))]
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
        let scope = if b.len() >= 28 { u32::from_ne_bytes([b[24], b[25], b[26], b[27]]) } else { 0 };
        Some(SocketAddr::with_scope(IpAddr::V6(ip), port, scope))
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

/// AF_UNIX (1 on Linux, Windows, macOS and iOS)
const AF_UNIX: i32 = 1;

/// a connected Unix domain stream socket (the stream calls of TcpStream)
pub type UnixStream = TcpStream;

/// sockaddr_un of a socket path on Apple: sun_len, sun_family, sun_path of 104 bytes
#[cfg(target_vendor = "apple")]
fn unix_addr(path: &str) -> NetResult<Vec<u8>> {
    let p = path.as_bytes();
    if p.is_empty() || p.len() >= 104 || p.contains(&0) {
        return Err(NetError::new(NetErrorKind::Other, 0));
    }
    let mut b = alloc::vec![0u8; 2 + 104];
    b[0] = b.len() as u8;
    b[1] = AF_UNIX as u8;
    b[2..2 + p.len()].copy_from_slice(p);
    Ok(b)
}

/// sockaddr_un of a socket path (sun_path holds 108 bytes on both systems)
#[cfg(not(target_vendor = "apple"))]
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
    // the socket file's (device, file id) after the bind: the drop removes only that file
    id: Option<(u64, u64)>,
}

impl UnixListener {
    /// listen on a socket file.  A socket file left at `path` by a listener that is gone (a
    /// connection to it is refused) is removed first; anything else there -- a file, a folder,
    /// a socket that is listened on -- stays, and bind fails (AddrInUse; a folder on Windows:
    /// Other).
    pub fn bind(path: &str) -> NetResult<UnixListener> {
        if !crate::policy::fs_allowed(path, crate::policy::FS_WRITE) {
            return Err(denied());
        }
        let a = unix_addr(path)?;
        let s = Sock(sys::socket(AF_UNIX, 1, 0)?);
        if let Err(e) = sys::bind_to(s.0, &a) {
            if e.kind != NetErrorKind::AddrInUse || !remove_stale(path, &a) {
                return Err(e);
            }
            sys::bind_to(s.0, &a)?;
        }
        sys::listen_on(s.0, 64)?;
        Ok(UnixListener { s, path: String::from(path), id: crate::fs::socket_id(path) })
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
        // only its own socket file: a name deleted and taken again meanwhile (a file, another
        // listener's socket) stays
        if self.id.is_some() && crate::fs::socket_id(&self.path) == self.id {
            crate::delete_raw(&self.path);
        }
    }
}

// a socket file at path that nobody listens on (a connection is refused at once) -> removed
fn remove_stale(path: &str, a: &[u8]) -> bool {
    let Some(id) = crate::fs::socket_id(path) else { return false };
    let refused = match sys::socket(AF_UNIX, 1, 0) {
        Ok(c) => {
            let c = Sock(c);
            // without blocking: a listener with a full backlog would hold a blocking connect
            sys::set_nonblocking(c.0, true).is_ok() && sys::connect_to(c.0, a).is_err_and(|e| e.kind == NetErrorKind::Refused)
        }
        Err(_) => false,
    };
    // still the same file (not replaced meanwhile)
    refused && crate::fs::socket_id(path) == Some(id) && crate::delete_raw(path)
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

/// the addresses of a host: numeric forms and "localhost" everywhere; on Windows, macOS and iOS
/// other names through the system resolver (Linux: netio::dns)
pub fn resolve(host: &str, port: u16) -> NetResult<Vec<SocketAddr>> {
    let h = host.trim_start_matches('[').trim_end_matches(']');
    if let Some((ip, scope)) = ip_scope_parse(h) {
        return Ok(alloc::vec![SocketAddr::with_scope(ip, port, scope)]);
    }
    if h.eq_ignore_ascii_case("localhost") {
        return Ok(alloc::vec![SocketAddr::new(IpAddr::LOCALHOST_V4, port), SocketAddr::new(IpAddr::LOCALHOST_V6, port)]);
    }
    #[cfg(any(windows, target_vendor = "apple"))]
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
    #[cfg(target_vendor = "apple")]
    {
        #[link(name = "System")]
        unsafe extern "C" {
            fn gethostname(name: *mut u8, len: usize) -> i32;
        }
        let mut buf = [0u8; 256];
        // SAFETY: buf holds 256 bytes; the name comes back NUL-terminated (or truncated).
        let ok = unsafe { gethostname(buf.as_mut_ptr(), buf.len() - 1) } == 0;
        let n = buf.iter().position(|&c| c == 0).unwrap_or(0);
        let name = String::from(String::from_utf8_lossy(&buf[..n]).trim());
        if ok && !name.is_empty() { name } else { String::from("localhost") }
    }
}

/// "host:port" with brackets around IPv6 literals
pub fn host_port(host: &str, port: u16) -> String {
    if host.contains(':') && !host.starts_with('[') { format!("[{}]:{}", host, port) } else { format!("{}:{}", host, port) }
}
