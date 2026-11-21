// ip -- the IP addresses of certificates (subject alternative names, name constraints), as
// plain octets with a parser and the text form of RFC 5952; the same shape as plat::net::IpAddr
// (this crate needs no platform), so `IpAddr::from_octets` and `octets` convert

use alloc::vec::Vec;
use core::fmt;

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

impl IpAddr {
    /// the address of 4 or 16 octets (None for another length)
    pub fn from_octets(b: &[u8]) -> Option<IpAddr> {
        match b.len() {
            4 => Some(IpAddr::V4([b[0], b[1], b[2], b[3]])),
            16 => {
                let mut a = [0u8; 16];
                a.copy_from_slice(b);
                Some(IpAddr::V6(a))
            }
            _ => None,
        }
    }

    /// its 4 or 16 octets
    pub fn octets(&self) -> &[u8] {
        match self {
            IpAddr::V4(a) => a,
            IpAddr::V6(a) => a,
        }
    }
}

impl From<[u8; 4]> for IpAddr {
    fn from(a: [u8; 4]) -> IpAddr {
        IpAddr::V4(a)
    }
}

impl From<[u8; 16]> for IpAddr {
    fn from(a: [u8; 16]) -> IpAddr {
        IpAddr::V6(a)
    }
}
