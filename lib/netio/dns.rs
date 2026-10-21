// dns -- host names to addresses
//
//   resolve(host, port) -> NetResult<Vec<SocketAddr>>   numeric addresses and "localhost"
//                     directly; Windows: the system resolver (plat); Linux: /etc/hosts, then
//                     the name servers of /etc/resolv.conf (search domains, ndots)
//   resolve_with(host, port, &Config)   the same with a given configuration (tests)
//   Config::system()                    /etc/hosts + /etc/resolv.conf
//
// Queries: A and AAAA over UDP (2 s per server and attempt, two rounds), over TCP when the answer
// is truncated; an answer must come from the server asked and carry the query's id and question.
// IPv4 addresses come first.  Not here: DNSSEC validation, mDNS, caching.
//
// The app's permissions: a name its net.connect rules do not cover fails with
// NetErrorKind::Denied before any query; the addresses of a covered name are registered
// (plat::policy::net_resolved), so connections to them follow that name's ports.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use plat::net::{IpAddr, NetError, NetErrorKind, NetResult, SocketAddr, TcpStream, UdpSocket};

const TYPE_A: u16 = 1;
const TYPE_AAAA: u16 = 28;
const TIMEOUT_MS: u32 = 2000;

#[derive(Clone, Debug, Default)]
pub struct Config {
    /// the text of a hosts file
    pub hosts: String,
    pub nameservers: Vec<SocketAddr>,
    pub search: Vec<String>,
    /// names with at least this many dots are tried as they are first
    pub ndots: usize,
}

impl Config {
    /// /etc/hosts and /etc/resolv.conf of this machine
    pub fn system() -> Config {
        let read = |p: &str| plat::read_file(p).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
        let mut c = Config::from_resolv_conf(&read("/etc/resolv.conf"));
        c.hosts = read("/etc/hosts");
        c
    }

    pub fn from_resolv_conf(text: &str) -> Config {
        let mut c = Config { ndots: 1, ..Config::default() };
        for line in text.lines() {
            let line = line.split(['#', ';']).next().unwrap_or("");
            let mut w = line.split_whitespace();
            match w.next() {
                Some("nameserver") => {
                    if let Some(ip) = w.next().and_then(|a| IpAddr::parse(a.split('%').next().unwrap_or(a))) {
                        if c.nameservers.len() < 3 {
                            c.nameservers.push(SocketAddr::new(ip, 53));
                        }
                    }
                }
                Some("search") | Some("domain") => c.search = w.map(|d| String::from(d.trim_end_matches('.'))).filter(|d| !d.is_empty()).collect(),
                Some("options") => {
                    for o in w {
                        if let Some(n) = o.strip_prefix("ndots:").and_then(|n| n.parse().ok()) {
                            c.ndots = n;
                        }
                    }
                }
                _ => {}
            }
        }
        if c.nameservers.is_empty() {
            c.nameservers.push(SocketAddr::new(IpAddr::LOCALHOST_V4, 53));
        }
        c
    }
}

/// the addresses of `name` in a hosts file (names compare case-insensitively)
pub fn hosts_lookup(hosts: &str, name: &str) -> Vec<IpAddr> {
    let name = name.trim_end_matches('.');
    let mut out = Vec::new();
    for line in hosts.lines() {
        let line = line.split('#').next().unwrap_or("");
        let mut w = line.split_whitespace();
        let Some(ip) = w.next().and_then(|a| IpAddr::parse(a.split('%').next().unwrap_or(a))) else { continue };
        if w.any(|n| n.trim_end_matches('.').eq_ignore_ascii_case(name)) && !out.contains(&ip) {
            out.push(ip);
        }
    }
    out
}

/// a query for one name and type
pub fn query(id: u16, name: &str, qtype: u16) -> Option<Vec<u8>> {
    let mut q = Vec::with_capacity(32 + name.len());
    q.extend_from_slice(&id.to_be_bytes());
    q.extend_from_slice(&[0x01, 0x00, 0, 1, 0, 0, 0, 0, 0, 0]); // recursion desired, one question
    for label in name.trim_end_matches('.').split('.') {
        if label.is_empty() || label.len() > 63 {
            return None;
        }
        q.push(label.len() as u8);
        q.extend_from_slice(label.as_bytes());
    }
    q.push(0);
    if q.len() > 12 + 255 {
        return None;
    }
    q.extend_from_slice(&qtype.to_be_bytes());
    q.extend_from_slice(&[0, 1]);
    Some(q)
}

/// skip a (possibly compressed) name
fn skip_name(m: &[u8], mut p: usize) -> Option<usize> {
    for _ in 0..128 {
        let l = *m.get(p)? as usize;
        if l == 0 {
            return Some(p + 1);
        }
        if l & 0xC0 == 0xC0 {
            m.get(p + 1)?;
            return Some(p + 2);
        }
        if l & 0xC0 != 0 {
            return None;
        }
        p += 1 + l;
    }
    None
}

/// what an answer says
#[derive(Debug, PartialEq, Eq)]
pub enum Answer {
    /// the addresses of the asked type (maybe none: the name exists without them)
    Found(Vec<IpAddr>),
    /// the name does not exist
    NoName,
    /// too large for UDP: ask over TCP
    Truncated,
    /// a server failure or an answer that does not fit the question
    Bad,
}

/// read an answer to `q` (the query it answers)
pub fn parse_answer(m: &[u8], q: &[u8], qtype: u16) -> Answer {
    if m.len() < 12 || m[..2] != q[..2] || m[2] & 0x80 == 0 {
        return Answer::Bad;
    }
    if m[2] & 0x02 != 0 {
        return Answer::Truncated;
    }
    let rcode = m[3] & 0x0F;
    if rcode == 3 {
        return Answer::NoName;
    }
    if rcode != 0 {
        return Answer::Bad;
    }
    let qd = u16::from_be_bytes([m[4], m[5]]) as usize;
    let an = u16::from_be_bytes([m[6], m[7]]) as usize;
    // the question must be ours
    if qd != 1 || m.get(12..q.len()).is_none_or(|qq| !qq.eq_ignore_ascii_case(&q[12..])) {
        return Answer::Bad;
    }
    let mut p = q.len();
    let mut out = Vec::new();
    for _ in 0..an {
        let Some(np) = skip_name(m, p) else { return Answer::Bad };
        let Some(h) = m.get(np..np + 10) else { return Answer::Bad };
        let (t, class, len) = (u16::from_be_bytes([h[0], h[1]]), u16::from_be_bytes([h[2], h[3]]), u16::from_be_bytes([h[8], h[9]]) as usize);
        let Some(data) = m.get(np + 10..np + 10 + len) else { return Answer::Bad };
        if class == 1 && t == qtype {
            match (t, len) {
                (TYPE_A, 4) => out.push(IpAddr::V4([data[0], data[1], data[2], data[3]])),
                (TYPE_AAAA, 16) => {
                    let mut a = [0u8; 16];
                    a.copy_from_slice(data);
                    out.push(IpAddr::V6(a));
                }
                _ => {}
            }
        }
        p = np + 10 + len;
    }
    Answer::Found(out)
}

fn ask_udp(server: &SocketAddr, q: &[u8], qtype: u16) -> Option<Answer> {
    let local = if matches!(server.ip, IpAddr::V6(_)) { SocketAddr::new(IpAddr::V6([0; 16]), 0) } else { SocketAddr::new(IpAddr::UNSPECIFIED_V4, 0) };
    let s = UdpSocket::bind(&local).ok()?;
    s.send_to(q, server).ok()?;
    let mut buf = [0u8; 1500];
    let deadline = plat::time_us() + TIMEOUT_MS as u64 * 1000;
    loop {
        let left = deadline.saturating_sub(plat::time_us()) / 1000;
        if left == 0 {
            return None;
        }
        let (n, from) = s.recv_from(&mut buf, left as u32).ok()?;
        // an answer from somewhere else, or to another question, is ignored
        if from != *server {
            continue;
        }
        match parse_answer(&buf[..n], q, qtype) {
            Answer::Bad if buf[..2] != q[..2] => continue,
            a => return Some(a),
        }
    }
}

fn ask_tcp(server: &SocketAddr, q: &[u8], qtype: u16) -> Option<Answer> {
    let s = TcpStream::connect(server, TIMEOUT_MS).ok()?;
    s.set_read_timeout(Some(TIMEOUT_MS));
    let mut framed = (q.len() as u16).to_be_bytes().to_vec();
    framed.extend_from_slice(q);
    s.write_all(&framed).ok()?;
    let mut len = [0u8; 2];
    s.read_exact(&mut len).ok()?;
    let mut m = alloc::vec![0u8; u16::from_be_bytes(len) as usize];
    s.read_exact(&mut m).ok()?;
    Some(parse_answer(&m, q, qtype))
}

/// one name over the name servers -> None: no server answered
fn lookup(cfg: &Config, name: &str, qtype: u16) -> Option<Answer> {
    let id = u16::from_le_bytes(plat::random_bytes());
    let q = query(id, name, qtype)?;
    for _round in 0..2 {
        for server in &cfg.nameservers {
            match ask_udp(server, &q, qtype) {
                Some(Answer::Truncated) => {
                    if let Some(a) = ask_tcp(server, &q, qtype) {
                        return Some(a);
                    }
                }
                Some(Answer::Bad) | None => {}
                Some(a) => return Some(a),
            }
        }
    }
    None
}

fn not_found() -> NetError {
    NetError::new(NetErrorKind::NotFound, 0)
}

pub fn resolve_with(host: &str, port: u16, cfg: &Config) -> NetResult<Vec<SocketAddr>> {
    let h = host.trim_start_matches('[').trim_end_matches(']');
    if let Some(ip) = IpAddr::parse(h) {
        return Ok(alloc::vec![SocketAddr::new(ip, port)]);
    }
    if h.eq_ignore_ascii_case("localhost") || h.to_ascii_lowercase().ends_with(".localhost") {
        return Ok(alloc::vec![SocketAddr::new(IpAddr::LOCALHOST_V4, port), SocketAddr::new(IpAddr::LOCALHOST_V6, port)]);
    }
    if h.is_empty() || h.len() > 253 || !h.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.' || b == b'_') {
        return Err(not_found());
    }
    // the app's permissions: only names of its net.connect rules are looked up
    if !plat::policy::net_name_allowed(h) {
        return Err(NetError::new(NetErrorKind::Denied, 0));
    }
    let mut ips = hosts_lookup(&cfg.hosts, h);
    if ips.is_empty() {
        // the names to try: as given first when it has enough dots (or ends with a dot)
        let absolute = h.ends_with('.');
        let mut names: Vec<String> = Vec::new();
        let dots = h.matches('.').count();
        if absolute || dots >= cfg.ndots {
            names.push(String::from(h.trim_end_matches('.')));
        }
        if !absolute {
            names.extend(cfg.search.iter().map(|d| format!("{}.{}", h, d)));
            if dots < cfg.ndots {
                names.push(String::from(h));
            }
        }
        let mut answered = false;
        for n in names {
            let mut found = Vec::new();
            for t in [TYPE_A, TYPE_AAAA] {
                match lookup(cfg, &n, t) {
                    Some(Answer::Found(v)) => {
                        answered = true;
                        found.extend(v);
                    }
                    Some(_) => answered = true,
                    None => {}
                }
            }
            if !found.is_empty() {
                ips = found;
                break;
            }
        }
        if ips.is_empty() {
            return Err(if answered { not_found() } else { NetError::new(NetErrorKind::TimedOut, 0) });
        }
    }
    ips.sort_by_key(|ip| matches!(ip, IpAddr::V6(_)));
    let addrs: Vec<SocketAddr> = ips.into_iter().map(|ip| SocketAddr::new(ip, port)).collect();
    plat::policy::net_resolved(h, &addrs); // connections to them follow the name's rules
    Ok(addrs)
}

/// the addresses of a host (see the top of this file)
pub fn resolve(host: &str, port: u16) -> NetResult<Vec<SocketAddr>> {
    if cfg!(windows) {
        return plat::net::resolve(host, port);
    }
    resolve_with(host, port, &Config::system())
}
