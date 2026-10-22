// addr -- link and network addresses: MAC, IPv4, IPv6, prefixes
//
//   Mac([u8; 6])        parse "aa:bb:cc:dd:ee:ff" / "aa-bb-.." / "aabb.ccdd.eeff"; Display with
//                       colons, cisco() with dots; broadcast / multicast / local bits; EUI-64
//   Ipv4([u8; 4])       strict dotted decimal (no leading zeros); masks <-> prefix lengths;
//                       class tests (private, loopback, link local, multicast, broadcast)
//   Ipv6([u8; 16])      every RFC 4291 text form in, RFC 5952 out; solicited-node and
//                       link-local (EUI-64) addresses; multicast MAC mapping; multicast
//                       scope, global unicast, the common prefix of two addresses
//   IpAddr              V4 | V6
//   Prefix4 / Prefix6   an address with a prefix length ("10.0.0.1/24"): network, broadcast,
//                       contains, host range (/31 and /32 per RFC 3021); masks and wildcards
//   wildcard_match      ACL matching with any (also non-contiguous) wildcard mask
//
// These are value types for packets, not OS socket addresses (those are plat::net's).

use alloc::format;
use alloc::string::String;
use core::fmt;

// ---------------------------------------------------------------- MAC

/// an IEEE 802 MAC-48 address
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Debug)]
pub struct Mac(pub [u8; 6]);

impl Mac {
    pub const ZERO: Mac = Mac([0; 6]);
    pub const BROADCAST: Mac = Mac([0xFF; 6]);
    /// the destination of IEEE 802.1D bridge PDUs (STP)
    pub const STP: Mac = Mac([0x01, 0x80, 0xC2, 0, 0, 0]);

    pub const fn new(a: u8, b: u8, c: u8, d: u8, e: u8, f: u8) -> Mac {
        Mac([a, b, c, d, e, f])
    }
    pub fn is_broadcast(&self) -> bool {
        self.0 == [0xFF; 6]
    }
    /// group bit set (broadcast included)
    pub fn is_multicast(&self) -> bool {
        self.0[0] & 1 != 0
    }
    pub fn is_unicast(&self) -> bool {
        !self.is_multicast()
    }
    /// locally administered (not a vendor address)
    pub fn is_local(&self) -> bool {
        self.0[0] & 2 != 0
    }
    pub fn is_zero(&self) -> bool {
        self.0 == [0; 6]
    }
    /// "aa:bb:cc:dd:ee:ff", "aa-bb-cc-dd-ee-ff", "aabb.ccdd.eeff" (Cisco) or 12 hex digits
    pub fn parse(s: &str) -> Option<Mac> {
        let s = s.trim();
        let mut out = [0u8; 6];
        let digits: String = if s.len() == 17 {
            let sep = s.as_bytes()[2];
            if sep != b':' && sep != b'-' {
                return None;
            }
            for (i, c) in s.bytes().enumerate() {
                if i % 3 == 2 && c != sep {
                    return None;
                }
            }
            s.chars().filter(|&c| c != sep as char).collect()
        } else if s.len() == 14 {
            let b = s.as_bytes();
            if b[4] != b'.' || b[9] != b'.' {
                return None;
            }
            s.chars().filter(|&c| c != '.').collect()
        } else if s.len() == 12 {
            String::from(s)
        } else {
            return None;
        };
        if digits.len() != 12 {
            return None;
        }
        let d = digits.as_bytes();
        for (i, o) in out.iter_mut().enumerate() {
            *o = (hex_val(d[2 * i])? << 4) | hex_val(d[2 * i + 1])?;
        }
        Some(Mac(out))
    }
    /// Cisco notation "aabb.ccdd.eeff"
    pub fn cisco(&self) -> String {
        let m = &self.0;
        format!("{:02x}{:02x}.{:02x}{:02x}.{:02x}{:02x}", m[0], m[1], m[2], m[3], m[4], m[5])
    }
    /// the multicast MAC of an IPv4 group (01:00:5e + low 23 bits, RFC 1112)
    pub fn of_ipv4_multicast(ip: Ipv4) -> Mac {
        Mac([0x01, 0x00, 0x5E, ip.0[1] & 0x7F, ip.0[2], ip.0[3]])
    }
    /// the multicast MAC of an IPv6 group (33:33 + low 32 bits, RFC 2464)
    pub fn of_ipv6_multicast(ip: Ipv6) -> Mac {
        Mac([0x33, 0x33, ip.0[12], ip.0[13], ip.0[14], ip.0[15]])
    }
    /// the modified EUI-64 interface identifier (RFC 4291 appendix A)
    pub fn eui64(&self) -> [u8; 8] {
        let m = &self.0;
        [m[0] ^ 0x02, m[1], m[2], 0xFF, 0xFE, m[3], m[4], m[5]]
    }
    /// the address as a number (for ranges and counters)
    pub fn to_u64(&self) -> u64 {
        self.0.iter().fold(0u64, |a, &b| (a << 8) | b as u64)
    }
    pub fn from_u64(v: u64) -> Mac {
        let b = v.to_be_bytes();
        Mac([b[2], b[3], b[4], b[5], b[6], b[7]])
    }
}

impl fmt::Display for Mac {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let m = &self.0;
        write!(f, "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}", m[0], m[1], m[2], m[3], m[4], m[5])
    }
}

fn hex_val(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

// ---------------------------------------------------------------- IPv4

/// an IPv4 address
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Debug)]
pub struct Ipv4(pub [u8; 4]);

impl Ipv4 {
    pub const UNSPECIFIED: Ipv4 = Ipv4([0; 4]);
    pub const BROADCAST: Ipv4 = Ipv4([255; 4]);
    pub const LOCALHOST: Ipv4 = Ipv4([127, 0, 0, 1]);

    pub const fn new(a: u8, b: u8, c: u8, d: u8) -> Ipv4 {
        Ipv4([a, b, c, d])
    }
    pub const fn from_u32(v: u32) -> Ipv4 {
        Ipv4(v.to_be_bytes())
    }
    pub const fn to_u32(&self) -> u32 {
        u32::from_be_bytes(self.0)
    }
    /// strict dotted decimal: four parts 0..255, no leading zeros, nothing else
    pub fn parse(s: &str) -> Option<Ipv4> {
        let mut out = [0u8; 4];
        let mut n = 0;
        for part in s.split('.') {
            if n == 4 {
                return None;
            }
            out[n] = dec_u8(part)?;
            n += 1;
        }
        if n == 4 { Some(Ipv4(out)) } else { None }
    }
    pub fn is_unspecified(&self) -> bool {
        self.0 == [0; 4]
    }
    pub fn is_loopback(&self) -> bool {
        self.0[0] == 127
    }
    /// 10/8, 172.16/12, 192.168/16 (RFC 1918)
    pub fn is_private(&self) -> bool {
        let o = &self.0;
        o[0] == 10 || (o[0] == 172 && o[1] & 0xF0 == 16) || (o[0] == 192 && o[1] == 168)
    }
    /// 169.254/16
    pub fn is_link_local(&self) -> bool {
        self.0[0] == 169 && self.0[1] == 254
    }
    /// 224/4
    pub fn is_multicast(&self) -> bool {
        self.0[0] & 0xF0 == 224
    }
    /// 255.255.255.255 (the limited broadcast)
    pub fn is_broadcast(&self) -> bool {
        self.0 == [255; 4]
    }
    /// the prefix length of a contiguous netmask ("255.255.255.0" -> 24), None otherwise
    pub fn mask_len(&self) -> Option<u8> {
        let v = self.to_u32();
        let len = v.leading_ones();
        if v.checked_shl(len).unwrap_or(0) == 0 { Some(len as u8) } else { None }
    }
    /// the netmask of a prefix length (0..=32)
    pub fn mask_of(len: u8) -> Ipv4 {
        Ipv4::from_u32(mask32(len))
    }
    /// IPv4-mapped IPv6 address ::ffff:a.b.c.d
    pub fn to_ipv6_mapped(&self) -> Ipv6 {
        let mut b = [0u8; 16];
        b[10] = 0xFF;
        b[11] = 0xFF;
        b[12..].copy_from_slice(&self.0);
        Ipv6(b)
    }
}

impl fmt::Display for Ipv4 {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}.{}.{}.{}", self.0[0], self.0[1], self.0[2], self.0[3])
    }
}

/// a decimal 0..=255 without sign or leading zero
fn dec_u8(s: &str) -> Option<u8> {
    let b = s.as_bytes();
    if b.is_empty() || b.len() > 3 || (b.len() > 1 && b[0] == b'0') {
        return None;
    }
    let mut v: u32 = 0;
    for &c in b {
        if !c.is_ascii_digit() {
            return None;
        }
        v = v * 10 + (c - b'0') as u32;
    }
    if v <= 255 { Some(v as u8) } else { None }
}

const fn mask32(len: u8) -> u32 {
    if len == 0 {
        0
    } else if len >= 32 {
        u32::MAX
    } else {
        u32::MAX << (32 - len as u32)
    }
}

/// ACL-style match: the bits that are 0 in `wildcard` must be equal (also non-contiguous)
pub fn wildcard_match(ip: Ipv4, addr: Ipv4, wildcard: Ipv4) -> bool {
    let care = !wildcard.to_u32();
    ip.to_u32() & care == addr.to_u32() & care
}

// ---------------------------------------------------------------- IPv6

/// an IPv6 address
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Debug)]
pub struct Ipv6(pub [u8; 16]);

impl Ipv6 {
    pub const UNSPECIFIED: Ipv6 = Ipv6([0; 16]);
    pub const LOCALHOST: Ipv6 = Ipv6([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]);
    /// ff02::1, all nodes on the link
    pub const ALL_NODES: Ipv6 = Ipv6([0xFF, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]);
    /// ff02::2, all routers on the link
    pub const ALL_ROUTERS: Ipv6 = Ipv6([0xFF, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2]);

    pub fn from_segments(s: [u16; 8]) -> Ipv6 {
        let mut b = [0u8; 16];
        for i in 0..8 {
            b[2 * i..2 * i + 2].copy_from_slice(&s[i].to_be_bytes());
        }
        Ipv6(b)
    }
    pub fn segments(&self) -> [u16; 8] {
        let mut s = [0u16; 8];
        for (i, v) in s.iter_mut().enumerate() {
            *v = u16::from_be_bytes([self.0[2 * i], self.0[2 * i + 1]]);
        }
        s
    }
    pub fn to_u128(&self) -> u128 {
        u128::from_be_bytes(self.0)
    }
    pub fn from_u128(v: u128) -> Ipv6 {
        Ipv6(v.to_be_bytes())
    }
    /// every RFC 4291 text form: full, "::" compressed, with an embedded IPv4 tail
    pub fn parse(s: &str) -> Option<Ipv6> {
        if s.is_empty() || s.len() > 45 {
            return None;
        }
        let (head, tail, compressed) = match s.find("::") {
            Some(p) => {
                if s[p + 2..].contains("::") {
                    return None;
                }
                (&s[..p], &s[p + 2..], true)
            }
            None => (s, "", false),
        };
        let mut front = [0u16; 8];
        let mut nf = 0;
        let mut back = [0u16; 8];
        let mut nb = 0;
        let parse_part = |part: &str, out: &mut [u16; 8], n: &mut usize, last_may_v4: bool| -> Option<()> {
            if part.is_empty() {
                return Some(());
            }
            let pieces: alloc::vec::Vec<&str> = part.split(':').collect();
            for (i, p) in pieces.iter().enumerate() {
                if last_may_v4 && i == pieces.len() - 1 && p.contains('.') {
                    let v4 = Ipv4::parse(p)?;
                    if *n + 2 > 8 {
                        return None;
                    }
                    out[*n] = u16::from_be_bytes([v4.0[0], v4.0[1]]);
                    out[*n + 1] = u16::from_be_bytes([v4.0[2], v4.0[3]]);
                    *n += 2;
                    continue;
                }
                if p.is_empty() || p.len() > 4 || *n >= 8 {
                    return None;
                }
                let mut v: u16 = 0;
                for &c in p.as_bytes() {
                    v = (v << 4) | hex_val(c)? as u16;
                }
                out[*n] = v;
                *n += 1;
            }
            Some(())
        };
        if compressed {
            parse_part(head, &mut front, &mut nf, false)?;
            parse_part(tail, &mut back, &mut nb, true)?;
            if nf + nb > 7 {
                return None;
            }
        } else {
            parse_part(head, &mut front, &mut nf, true)?;
            if nf != 8 {
                return None;
            }
        }
        let mut seg = [0u16; 8];
        seg[..nf].copy_from_slice(&front[..nf]);
        seg[8 - nb..].copy_from_slice(&back[..nb]);
        Some(Ipv6::from_segments(seg))
    }
    pub fn is_unspecified(&self) -> bool {
        self.0 == [0; 16]
    }
    pub fn is_loopback(&self) -> bool {
        *self == Ipv6::LOCALHOST
    }
    /// ff00::/8
    pub fn is_multicast(&self) -> bool {
        self.0[0] == 0xFF
    }
    /// fe80::/10
    pub fn is_link_local(&self) -> bool {
        self.0[0] == 0xFE && self.0[1] & 0xC0 == 0x80
    }
    /// fc00::/7
    pub fn is_unique_local(&self) -> bool {
        self.0[0] & 0xFE == 0xFC
    }
    /// ::ffff:a.b.c.d -> a.b.c.d
    pub fn to_ipv4_mapped(&self) -> Option<Ipv4> {
        if self.0[..10] == [0; 10] && self.0[10] == 0xFF && self.0[11] == 0xFF {
            Some(Ipv4([self.0[12], self.0[13], self.0[14], self.0[15]]))
        } else {
            None
        }
    }
    /// the solicited-node multicast address ff02::1:ffXX:XXXX (RFC 4291 2.7.1)
    pub fn solicited_node(&self) -> Ipv6 {
        let mut b = [0u8; 16];
        b[0] = 0xFF;
        b[1] = 0x02;
        b[11] = 0x01;
        b[12] = 0xFF;
        b[13..].copy_from_slice(&self.0[13..]);
        Ipv6(b)
    }
    /// fe80::/64 with the modified EUI-64 of a MAC
    pub fn link_local_of(mac: Mac) -> Ipv6 {
        let mut b = [0u8; 16];
        b[0] = 0xFE;
        b[1] = 0x80;
        b[8..].copy_from_slice(&mac.eui64());
        Ipv6(b)
    }
    /// a /64 prefix plus the modified EUI-64 of a MAC (SLAAC, RFC 4862)
    pub fn eui64_in(prefix: Ipv6, mac: Mac) -> Ipv6 {
        let mut b = prefix.0;
        b[8..].copy_from_slice(&mac.eui64());
        Ipv6(b)
    }
    /// ff02::1:ff00:0/104, a solicited-node multicast address
    pub fn is_solicited_node(&self) -> bool {
        self.0[..13] == [0xFF, 0x02, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x01, 0xFF]
    }
    /// the scope of a multicast address (1 interface, 2 link, 5 site, 8 organisation, 14
    /// global; RFC 4291 2.7)
    pub fn mc_scope(&self) -> u8 {
        self.0[1] & 0x0F
    }
    /// a global unicast address (not unspecified, loopback, link-local, multicast; unique
    /// local addresses count as global in scope, RFC 4193)
    pub fn is_global_unicast(&self) -> bool {
        !self.is_unspecified() && !self.is_loopback() && !self.is_link_local() && !self.is_multicast() && self.to_ipv4_mapped().is_none()
    }
    /// the number of leading bits two addresses share (source address selection, RFC 6724)
    pub fn common_prefix(&self, other: &Ipv6) -> u8 {
        (self.to_u128() ^ other.to_u128()).leading_zeros() as u8
    }
}

impl fmt::Display for Ipv6 {
    /// RFC 5952: lower case, no leading zeros, the longest run (first on a tie) of two or more
    /// zero groups as "::", IPv4-mapped addresses with a dotted tail
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if let Some(v4) = self.to_ipv4_mapped() {
            return write!(f, "::ffff:{}", v4);
        }
        let s = self.segments();
        let (mut best, mut best_len) = (8usize, 0usize);
        let mut i = 0;
        while i < 8 {
            if s[i] == 0 {
                let start = i;
                while i < 8 && s[i] == 0 {
                    i += 1;
                }
                if i - start > best_len {
                    best = start;
                    best_len = i - start;
                }
            } else {
                i += 1;
            }
        }
        if best_len < 2 {
            best = 8;
            best_len = 0;
        }
        let mut i = 0;
        while i < 8 {
            if i == best {
                f.write_str("::")?;
                i += best_len;
                continue;
            }
            if i > 0 && i != best + best_len {
                f.write_str(":")?;
            }
            write!(f, "{:x}", s[i])?;
            i += 1;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------- IpAddr

/// an IPv4 or IPv6 address
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum IpAddr {
    V4(Ipv4),
    V6(Ipv6),
}

impl IpAddr {
    pub fn parse(s: &str) -> Option<IpAddr> {
        if s.contains(':') {
            Ipv6::parse(s).map(IpAddr::V6)
        } else {
            Ipv4::parse(s).map(IpAddr::V4)
        }
    }
    pub fn is_v4(&self) -> bool {
        matches!(self, IpAddr::V4(_))
    }
    pub fn v4(&self) -> Option<Ipv4> {
        match self {
            IpAddr::V4(a) => Some(*a),
            IpAddr::V6(_) => None,
        }
    }
    pub fn v6(&self) -> Option<Ipv6> {
        match self {
            IpAddr::V6(a) => Some(*a),
            IpAddr::V4(_) => None,
        }
    }
    pub fn is_unspecified(&self) -> bool {
        match self {
            IpAddr::V4(a) => a.is_unspecified(),
            IpAddr::V6(a) => a.is_unspecified(),
        }
    }
    pub fn is_multicast(&self) -> bool {
        match self {
            IpAddr::V4(a) => a.is_multicast(),
            IpAddr::V6(a) => a.is_multicast(),
        }
    }
}

impl From<Ipv4> for IpAddr {
    fn from(a: Ipv4) -> IpAddr {
        IpAddr::V4(a)
    }
}

impl From<Ipv6> for IpAddr {
    fn from(a: Ipv6) -> IpAddr {
        IpAddr::V6(a)
    }
}

impl fmt::Display for IpAddr {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            IpAddr::V4(a) => a.fmt(f),
            IpAddr::V6(a) => a.fmt(f),
        }
    }
}

// ---------------------------------------------------------------- prefixes

/// an IPv4 address with a prefix length; the address keeps its host bits ("10.0.0.1/24" is an
/// interface address, network() is 10.0.0.0)
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Debug)]
pub struct Prefix4 {
    pub addr: Ipv4,
    pub len: u8,
}

impl Prefix4 {
    pub fn new(addr: Ipv4, len: u8) -> Option<Prefix4> {
        if len <= 32 { Some(Prefix4 { addr, len }) } else { None }
    }
    /// "a.b.c.d/len" or "a.b.c.d" (= /32)
    pub fn parse(s: &str) -> Option<Prefix4> {
        match s.split_once('/') {
            Some((a, l)) => {
                let lb = l.as_bytes();
                if lb.is_empty() || lb.len() > 2 || !lb.iter().all(|c| c.is_ascii_digit()) || (lb.len() == 2 && lb[0] == b'0') {
                    return None;
                }
                Prefix4::new(Ipv4::parse(a)?, l.parse().ok()?)
            }
            None => Some(Prefix4 { addr: Ipv4::parse(s)?, len: 32 }),
        }
    }
    /// an address and a dotted netmask ("255.255.255.0"); None for a non-contiguous mask
    pub fn from_mask(addr: Ipv4, mask: Ipv4) -> Option<Prefix4> {
        Some(Prefix4 { addr, len: mask.mask_len()? })
    }
    /// an address and a contiguous wildcard mask ("0.0.0.255")
    pub fn from_wildcard(addr: Ipv4, wildcard: Ipv4) -> Option<Prefix4> {
        Prefix4::from_mask(addr, Ipv4::from_u32(!wildcard.to_u32()))
    }
    pub fn mask(&self) -> Ipv4 {
        Ipv4::mask_of(self.len)
    }
    pub fn wildcard(&self) -> Ipv4 {
        Ipv4::from_u32(!mask32(self.len))
    }
    /// the network address (host bits cleared)
    pub fn network(&self) -> Ipv4 {
        Ipv4::from_u32(self.addr.to_u32() & mask32(self.len))
    }
    /// this prefix with the host bits cleared
    pub fn trunc(&self) -> Prefix4 {
        Prefix4 { addr: self.network(), len: self.len }
    }
    /// the directed broadcast address (all host bits set)
    pub fn broadcast(&self) -> Ipv4 {
        Ipv4::from_u32(self.addr.to_u32() | !mask32(self.len))
    }
    pub fn contains(&self, ip: Ipv4) -> bool {
        let m = mask32(self.len);
        ip.to_u32() & m == self.addr.to_u32() & m
    }
    /// `other` lies completely inside this prefix
    pub fn covers(&self, other: &Prefix4) -> bool {
        other.len >= self.len && self.contains(other.addr)
    }
    /// two prefixes share addresses
    pub fn overlaps(&self, other: &Prefix4) -> bool {
        self.contains(other.addr) || other.contains(self.addr)
    }
    /// the address is the network or broadcast address of a /0../30 prefix
    pub fn is_reserved_host(&self, ip: Ipv4) -> bool {
        self.len <= 30 && (ip == self.network() || ip == self.broadcast())
    }
    /// usable host addresses (/31: both, RFC 3021; /32: one)
    pub fn host_count(&self) -> u64 {
        match self.len {
            32 => 1,
            31 => 2,
            l => (1u64 << (32 - l as u32)) - 2,
        }
    }
    pub fn first_host(&self) -> Ipv4 {
        if self.len >= 31 { self.network() } else { Ipv4::from_u32(self.network().to_u32() + 1) }
    }
    pub fn last_host(&self) -> Ipv4 {
        if self.len >= 31 { self.broadcast() } else { Ipv4::from_u32(self.broadcast().to_u32() - 1) }
    }
}

impl fmt::Display for Prefix4 {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}/{}", self.addr, self.len)
    }
}

/// an IPv6 address with a prefix length
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Debug)]
pub struct Prefix6 {
    pub addr: Ipv6,
    pub len: u8,
}

impl Prefix6 {
    pub fn new(addr: Ipv6, len: u8) -> Option<Prefix6> {
        if len <= 128 { Some(Prefix6 { addr, len }) } else { None }
    }
    /// "2001:db8::1/64" or an address alone (= /128)
    pub fn parse(s: &str) -> Option<Prefix6> {
        match s.split_once('/') {
            Some((a, l)) => {
                let lb = l.as_bytes();
                if lb.is_empty() || lb.len() > 3 || !lb.iter().all(|c| c.is_ascii_digit()) || (lb.len() > 1 && lb[0] == b'0') {
                    return None;
                }
                Prefix6::new(Ipv6::parse(a)?, l.parse().ok()?)
            }
            None => Some(Prefix6 { addr: Ipv6::parse(s)?, len: 128 }),
        }
    }
    fn mask(&self) -> u128 {
        if self.len == 0 { 0 } else { u128::MAX << (128 - self.len as u32) }
    }
    pub fn network(&self) -> Ipv6 {
        Ipv6::from_u128(self.addr.to_u128() & self.mask())
    }
    pub fn trunc(&self) -> Prefix6 {
        Prefix6 { addr: self.network(), len: self.len }
    }
    pub fn contains(&self, ip: Ipv6) -> bool {
        let m = self.mask();
        ip.to_u128() & m == self.addr.to_u128() & m
    }
    pub fn covers(&self, other: &Prefix6) -> bool {
        other.len >= self.len && self.contains(other.addr)
    }
    /// two prefixes share addresses
    pub fn overlaps(&self, other: &Prefix6) -> bool {
        self.contains(other.addr) || other.contains(self.addr)
    }
}

impl fmt::Display for Prefix6 {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}/{}", self.addr, self.len)
    }
}
