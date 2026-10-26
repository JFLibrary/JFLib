// ipv4 -- the IPv4 header (RFC 791), fragmentation and reassembly
//
//   Ipv4View::parse(b)     version 4, IHL >= 5, total length within the buffer (trailing bytes,
//                          e.g. Ethernet padding, are ignored by payload())
//   Ipv4Hdr { .. }         write(out, payload_len) fills length and checksum; push_onto(&mut Pkt)
//   dec_ttl(hdr) -> u8     TTL - 1 in place with an incremental checksum update (RFC 1624)
//   fragment(pkt, mtu)     splits a packet into fragments of at most `mtu` bytes (None when DF
//                          is set and the packet is too large); options with the copy flag go
//                          into every fragment
//   Reassembly             collects fragments of one datagram (no timers: the caller expires
//                          it after its own timeout)

use alloc::collections::BTreeMap;
use alloc::vec::Vec;

use crate::addr::Ipv4;
use crate::checksum;
use crate::pkt::Pkt;
use crate::{be16, put16};

pub const IPV4_MIN: usize = 20;
pub const IPV4_MAX_OPTS: usize = 40;

pub const PROTO_ICMP: u8 = 1;
pub const PROTO_IGMP: u8 = 2;
pub const PROTO_TCP: u8 = 6;
pub const PROTO_UDP: u8 = 17;
pub const PROTO_IPV6: u8 = 41;
pub const PROTO_GRE: u8 = 47;
pub const PROTO_ESP: u8 = 50;
pub const PROTO_AH: u8 = 51;
pub const PROTO_ICMPV6: u8 = 58;
pub const PROTO_OSPF: u8 = 89;
pub const PROTO_VRRP: u8 = 112;

/// a name for an IP protocol number ("TCP", "UDP", ...)
pub fn proto_name(p: u8) -> Option<&'static str> {
    Some(match p {
        PROTO_ICMP => "ICMP",
        PROTO_IGMP => "IGMP",
        PROTO_TCP => "TCP",
        PROTO_UDP => "UDP",
        PROTO_IPV6 => "IPv6",
        PROTO_GRE => "GRE",
        PROTO_ESP => "ESP",
        PROTO_AH => "AH",
        PROTO_ICMPV6 => "ICMPv6",
        PROTO_OSPF => "OSPF",
        PROTO_VRRP => "VRRP",
        _ => return None,
    })
}

/// a received IPv4 packet
#[derive(Clone, Copy, Debug)]
pub struct Ipv4View<'a> {
    b: &'a [u8],
    end: usize,
}

impl<'a> Ipv4View<'a> {
    pub fn parse(b: &'a [u8]) -> Option<Ipv4View<'a>> {
        let v = Ipv4View::parse_truncated(b)?;
        if v.end < v.total_len() { None } else { Some(v) }
    }
    /// a packet that may be cut after its header (the quote inside an ICMP error): payload()
    /// then ends at the end of the buffer
    pub fn parse_truncated(b: &'a [u8]) -> Option<Ipv4View<'a>> {
        if b.len() < IPV4_MIN || b[0] >> 4 != 4 {
            return None;
        }
        let ihl = (b[0] & 0x0F) as usize * 4;
        let total = be16(b, 2) as usize;
        if ihl < IPV4_MIN || total < ihl || ihl > b.len() {
            return None;
        }
        Some(Ipv4View { b, end: total.min(b.len()) })
    }
    /// fewer bytes than the total length are present
    pub fn is_truncated(&self) -> bool {
        self.end < self.total_len()
    }
    pub fn header_len(&self) -> usize {
        (self.b[0] & 0x0F) as usize * 4
    }
    pub fn dscp(&self) -> u8 {
        self.b[1] >> 2
    }
    pub fn ecn(&self) -> u8 {
        self.b[1] & 3
    }
    pub fn total_len(&self) -> usize {
        be16(self.b, 2) as usize
    }
    pub fn id(&self) -> u16 {
        be16(self.b, 4)
    }
    pub fn df(&self) -> bool {
        self.b[6] & 0x40 != 0
    }
    pub fn mf(&self) -> bool {
        self.b[6] & 0x20 != 0
    }
    /// the fragment offset in bytes
    pub fn frag_off(&self) -> usize {
        (be16(self.b, 6) & 0x1FFF) as usize * 8
    }
    /// this packet is a fragment (MF set or offset > 0)
    pub fn is_fragment(&self) -> bool {
        self.mf() || self.frag_off() > 0
    }
    pub fn ttl(&self) -> u8 {
        self.b[8]
    }
    pub fn proto(&self) -> u8 {
        self.b[9]
    }
    pub fn checksum(&self) -> u16 {
        be16(self.b, 10)
    }
    pub fn checksum_ok(&self) -> bool {
        checksum::verify(self.header())
    }
    pub fn src(&self) -> Ipv4 {
        Ipv4([self.b[12], self.b[13], self.b[14], self.b[15]])
    }
    pub fn dst(&self) -> Ipv4 {
        Ipv4([self.b[16], self.b[17], self.b[18], self.b[19]])
    }
    pub fn options(&self) -> &'a [u8] {
        &self.b[IPV4_MIN..self.header_len()]
    }
    pub fn header(&self) -> &'a [u8] {
        &self.b[..self.header_len()]
    }
    /// the payload up to the total length
    pub fn payload(&self) -> &'a [u8] {
        &self.b[self.header_len()..self.end]
    }
    /// header and payload, without trailing padding
    pub fn packet(&self) -> &'a [u8] {
        &self.b[..self.end]
    }
}

/// an IPv4 header to write
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ipv4Hdr {
    pub dscp: u8,
    pub ecn: u8,
    pub id: u16,
    pub df: bool,
    pub mf: bool,
    /// fragment offset in bytes (a multiple of 8)
    pub frag_off: u16,
    pub ttl: u8,
    pub proto: u8,
    pub src: Ipv4,
    pub dst: Ipv4,
    pub opts_len: u8,
    pub opts: [u8; IPV4_MAX_OPTS],
}

impl Default for Ipv4Hdr {
    fn default() -> Ipv4Hdr {
        Ipv4Hdr {
            dscp: 0,
            ecn: 0,
            id: 0,
            df: false,
            mf: false,
            frag_off: 0,
            ttl: 64,
            proto: 0,
            src: Ipv4::UNSPECIFIED,
            dst: Ipv4::UNSPECIFIED,
            opts_len: 0,
            opts: [0; IPV4_MAX_OPTS],
        }
    }
}

impl Ipv4Hdr {
    pub fn new(src: Ipv4, dst: Ipv4, proto: u8, ttl: u8) -> Ipv4Hdr {
        Ipv4Hdr { src, dst, proto, ttl, ..Ipv4Hdr::default() }
    }
    pub fn read(v: &Ipv4View) -> Ipv4Hdr {
        let o = v.options();
        let mut opts = [0u8; IPV4_MAX_OPTS];
        opts[..o.len()].copy_from_slice(o);
        Ipv4Hdr {
            dscp: v.dscp(),
            ecn: v.ecn(),
            id: v.id(),
            df: v.df(),
            mf: v.mf(),
            frag_off: v.frag_off() as u16,
            ttl: v.ttl(),
            proto: v.proto(),
            src: v.src(),
            dst: v.dst(),
            opts_len: o.len() as u8,
            opts,
        }
    }
    /// sets the options (padded with zeros to a multiple of 4; at most 40 bytes)
    pub fn set_options(&mut self, o: &[u8]) -> bool {
        let n = (o.len() + 3) & !3;
        if n > IPV4_MAX_OPTS {
            return false;
        }
        self.opts = [0; IPV4_MAX_OPTS];
        self.opts[..o.len()].copy_from_slice(o);
        self.opts_len = n as u8;
        true
    }
    pub fn len(&self) -> usize {
        IPV4_MIN + self.opts_len as usize
    }
    /// writes the header for a payload of `payload_len` bytes, with total length and checksum
    pub fn write(&self, out: &mut [u8], payload_len: usize) {
        let hl = self.len();
        out[0] = 0x40 | (hl / 4) as u8;
        out[1] = (self.dscp << 2) | (self.ecn & 3);
        put16(out, 2, (hl + payload_len) as u16);
        put16(out, 4, self.id);
        let mut fo = (self.frag_off / 8) & 0x1FFF;
        if self.df {
            fo |= 0x4000;
        }
        if self.mf {
            fo |= 0x2000;
        }
        put16(out, 6, fo);
        out[8] = self.ttl;
        out[9] = self.proto;
        put16(out, 10, 0);
        out[12..16].copy_from_slice(&self.src.0);
        out[16..20].copy_from_slice(&self.dst.0);
        out[IPV4_MIN..hl].copy_from_slice(&self.opts[..self.opts_len as usize]);
        let ck = checksum::checksum(&out[..hl]);
        put16(out, 10, ck);
    }
    /// prepends the header; the packet's current content is the payload
    pub fn push_onto(&self, p: &mut Pkt) {
        let pl = p.len();
        self.write(p.push(self.len()), pl);
    }
}

/// decrements the TTL of a header in place and updates the checksum incrementally; returns
/// the new TTL (0 stays 0)
pub fn dec_ttl(hdr: &mut [u8]) -> u8 {
    if hdr[8] == 0 {
        return 0;
    }
    let old = u16::from_be_bytes([hdr[8], hdr[9]]);
    hdr[8] -= 1;
    let new = u16::from_be_bytes([hdr[8], hdr[9]]);
    let ck = checksum::update16(be16(hdr, 10), old, new);
    put16(hdr, 10, ck);
    hdr[8]
}

/// the options of a header that are copied into every fragment (copy flag set, RFC 791)
fn copied_options(opts: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < opts.len() {
        let t = opts[i];
        if t == 0 {
            break;
        }
        if t == 1 {
            i += 1;
            continue;
        }
        if i + 1 >= opts.len() {
            break;
        }
        let l = opts[i + 1] as usize;
        if l < 2 || i + l > opts.len() {
            break;
        }
        if t & 0x80 != 0 {
            out.extend_from_slice(&opts[i..i + l]);
        }
        i += l;
    }
    out
}

/// splits an IPv4 packet into fragments of at most `mtu` bytes; the packet itself when it
/// fits, None when it does not fit and DF is set (the sender then gets "fragmentation needed")
pub fn fragment(packet: &[u8], mtu: usize) -> Option<Vec<Vec<u8>>> {
    let v = Ipv4View::parse(packet)?;
    if v.total_len() <= mtu {
        return Some(alloc::vec![v.packet().to_vec()]);
    }
    if v.df() {
        return None;
    }
    let first = Ipv4Hdr::read(&v);
    let mut later = first;
    later.set_options(&copied_options(v.options()));
    let data = v.payload();
    let base_off = v.frag_off();
    if base_off + data.len() > Reassembly::MAX {
        return None;
    }
    let last_mf = v.mf();
    let mut out = Vec::new();
    let mut pos = 0;
    while pos < data.len() {
        let mut h = if pos == 0 { first } else { later };
        let room = mtu.checked_sub(h.len())? & !7;
        if room == 0 {
            return None;
        }
        let n = room.min(data.len() - pos);
        h.frag_off = (base_off + pos) as u16;
        h.mf = pos + n < data.len() || last_mf;
        let mut f = alloc::vec![0u8; h.len() + n];
        h.write(&mut f, n);
        let hl = h.len();
        f[hl..].copy_from_slice(&data[pos..pos + n]);
        out.push(f);
        pos += n;
    }
    Some(out)
}

/// the fragments of one datagram (key: source, destination, protocol, id)
#[derive(Clone, Debug, Default)]
pub struct Reassembly {
    first: Option<Vec<u8>>,
    parts: BTreeMap<usize, Vec<u8>>,
    total: Option<usize>,
    bytes: usize,
}

/// the reassembly key of a fragment
pub fn reassembly_key(v: &Ipv4View) -> (Ipv4, Ipv4, u8, u16) {
    (v.src(), v.dst(), v.proto(), v.id())
}

impl Reassembly {
    /// the largest datagram accepted
    pub const MAX: usize = 65_535;

    pub fn new() -> Reassembly {
        Reassembly::default()
    }
    /// adds a fragment; Some(the complete packet) when all parts are there. A fragment that
    /// overlaps one already held (a duplicate too) is ignored.
    pub fn add(&mut self, v: &Ipv4View) -> Option<Vec<u8>> {
        let off = v.frag_off();
        let data = v.payload();
        let end = off + data.len();
        if end > Reassembly::MAX || data.is_empty() || (v.mf() && data.len() % 8 != 0) {
            return None;
        }
        let total = if v.mf() { self.total } else { Some(end) };
        if let Some(t) = total {
            if end > t || self.total.is_some_and(|s| s != t) {
                return None;
            }
            if self.parts.iter().any(|(&o, d)| o + d.len() > t) {
                return None;
            }
        }
        for (&o, d) in self.parts.iter() {
            if o < end && off < o + d.len() {
                return None;
            }
        }
        self.total = total;
        if off == 0 {
            self.first = Some(v.header().to_vec());
        }
        self.bytes += data.len();
        self.parts.insert(off, data.to_vec());
        let total = self.total?;
        if self.bytes != total {
            return None;
        }
        let first = self.first.as_ref()?;
        let mut h = Ipv4Hdr::read(&Ipv4View::parse(&{
            let mut x = first.clone();
            put16(&mut x, 2, first.len() as u16);
            x
        })?);
        h.mf = false;
        h.frag_off = 0;
        if h.len() + total > Reassembly::MAX {
            *self = Reassembly::new();
            return None;
        }
        let mut out = alloc::vec![0u8; h.len() + total];
        h.write(&mut out, total);
        let hl = h.len();
        for (&o, d) in self.parts.iter() {
            out[hl + o..hl + o + d.len()].copy_from_slice(d);
        }
        Some(out)
    }
    /// bytes collected so far
    pub fn collected(&self) -> usize {
        self.bytes
    }
}
