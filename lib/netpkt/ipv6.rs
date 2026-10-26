// ipv6 -- the IPv6 fixed header (RFC 8200), the walk over extension headers, the fragment
// header, fragmentation and reassembly
//
//   Ipv6View::parse(b)    version 6, payload length within the buffer
//   Ipv6View::parse_truncated(b)   a packet that may be cut after its header (the quote inside
//                         an ICMPv6 error): payload() ends at the end of the buffer
//   Ipv6Hdr { .. }        write(out, payload_len), push_onto(&mut Pkt)
//   upper_layer(v)        (next header, offset in the payload) after hop-by-hop, routing,
//                         fragment and destination options headers
//   ext_headers(v)        the extension headers in order (type, offset, length, next header);
//                         None for a malformed chain
//   FragHdr / frag(v)     the fragment header (offset in bytes, more fragments, identification)
//                         and where it sits
//   fragment(pkt, mtu, id)  fragments of at most `mtu` bytes with a fragment header (RFC 8200
//                         4.5: the unfragmentable part -- the header, hop-by-hop and routing
//                         headers -- in front of each); None when even one 8-byte unit does
//                         not fit
//   Reassembly6           collects the fragments of one packet; overlapping fragments drop the
//                         whole packet (RFC 5722); no timers: the caller expires it after 60 s
//   dec_hop_limit(pkt)    hop limit - 1 in place (IPv6 has no header checksum)
//
// ICMPv6 and Neighbor Discovery are in icmpv6.rs; the addresses of ND (solicited-node groups,
// EUI-64 interface identifiers, multicast MACs) in addr.rs.

use alloc::collections::BTreeMap;
use alloc::vec::Vec;

use crate::addr::Ipv6;
use crate::pkt::Pkt;
use crate::{be16, be32, put16, put32};

pub const IPV6_HDR: usize = 40;
/// the minimum link MTU of IPv6 (RFC 8200 5)
pub const MIN_MTU: usize = 1280;
/// the length of the fragment header
pub const FRAG_HDR: usize = 8;

pub const NH_HOPOPT: u8 = 0;
pub const NH_TCP: u8 = 6;
pub const NH_UDP: u8 = 17;
pub const NH_ROUTING: u8 = 43;
pub const NH_FRAGMENT: u8 = 44;
pub const NH_ESP: u8 = 50;
pub const NH_AH: u8 = 51;
pub const NH_ICMPV6: u8 = 58;
pub const NH_NONE: u8 = 59;
pub const NH_DSTOPTS: u8 = 60;

/// a name for an extension header type ("Hop-by-Hop Options", "Fragment", ...)
pub fn ext_name(nh: u8) -> Option<&'static str> {
    Some(match nh {
        NH_HOPOPT => "Hop-by-Hop Options",
        NH_ROUTING => "Routing Header",
        NH_FRAGMENT => "Fragment Header",
        NH_DSTOPTS => "Destination Options",
        NH_AH => "Authentication Header",
        _ => return None,
    })
}

/// a received IPv6 packet
#[derive(Clone, Copy, Debug)]
pub struct Ipv6View<'a> {
    b: &'a [u8],
    /// the end of the packet in the buffer (header + payload length, or the buffer's end)
    end: usize,
}

impl<'a> Ipv6View<'a> {
    pub fn parse(b: &'a [u8]) -> Option<Ipv6View<'a>> {
        let v = Ipv6View::parse_truncated(b)?;
        if v.is_truncated() { None } else { Some(v) }
    }
    /// a packet that may be cut after its header (the quote inside an ICMPv6 error)
    pub fn parse_truncated(b: &'a [u8]) -> Option<Ipv6View<'a>> {
        if b.len() < IPV6_HDR || b[0] >> 4 != 6 {
            return None;
        }
        Some(Ipv6View { b, end: (IPV6_HDR + be16(b, 4) as usize).min(b.len()) })
    }
    /// fewer bytes than the payload length are present
    pub fn is_truncated(&self) -> bool {
        self.end < IPV6_HDR + self.payload_len()
    }
    pub fn traffic_class(&self) -> u8 {
        ((self.b[0] & 0x0F) << 4) | (self.b[1] >> 4)
    }
    pub fn flow_label(&self) -> u32 {
        (((self.b[1] & 0x0F) as u32) << 16) | be16(self.b, 2) as u32
    }
    pub fn payload_len(&self) -> usize {
        be16(self.b, 4) as usize
    }
    pub fn next_header(&self) -> u8 {
        self.b[6]
    }
    pub fn hop_limit(&self) -> u8 {
        self.b[7]
    }
    pub fn src(&self) -> Ipv6 {
        Ipv6(self.b[8..24].try_into().unwrap_or([0; 16]))
    }
    pub fn dst(&self) -> Ipv6 {
        Ipv6(self.b[24..40].try_into().unwrap_or([0; 16]))
    }
    /// the payload up to the payload length (or the end of a truncated quote)
    pub fn payload(&self) -> &'a [u8] {
        &self.b[IPV6_HDR..self.end]
    }
    /// header and payload, without trailing padding
    pub fn packet(&self) -> &'a [u8] {
        &self.b[..self.end]
    }
}

/// the upper-layer protocol and its offset in the payload, after the extension headers;
/// None for a malformed chain
pub fn upper_layer(v: &Ipv6View) -> Option<(u8, usize)> {
    let p = v.payload();
    let mut nh = v.next_header();
    let mut off = 0;
    loop {
        match nh {
            NH_HOPOPT | NH_ROUTING | NH_DSTOPTS => {
                if off + 2 > p.len() {
                    return None;
                }
                let next = p[off];
                off += (p[off + 1] as usize + 1) * 8;
                nh = next;
            }
            NH_AH => {
                if off + 2 > p.len() {
                    return None;
                }
                let next = p[off];
                off += (p[off + 1] as usize + 2) * 4;
                nh = next;
            }
            NH_FRAGMENT => {
                if off + 8 > p.len() {
                    return None;
                }
                let next = p[off];
                off += 8;
                nh = next;
            }
            _ => break,
        }
        if off > p.len() {
            return None;
        }
    }
    Some((nh, off))
}

/// one extension header of a packet
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExtHdr {
    /// its type (the Next Header value that announced it)
    pub kind: u8,
    /// offset and length in the payload
    pub off: usize,
    pub len: usize,
    /// the Next Header field it carries
    pub next: u8,
}

/// the extension headers of a packet in order; None for a malformed chain
pub fn ext_headers(v: &Ipv6View) -> Option<Vec<ExtHdr>> {
    let p = v.payload();
    let mut out = Vec::new();
    let mut nh = v.next_header();
    let mut off = 0;
    while matches!(nh, NH_HOPOPT | NH_ROUTING | NH_DSTOPTS | NH_FRAGMENT) {
        if off + 2 > p.len() {
            return None;
        }
        let len = if nh == NH_FRAGMENT { FRAG_HDR } else { (p[off + 1] as usize + 1) * 8 };
        if off + len > p.len() {
            return None;
        }
        out.push(ExtHdr { kind: nh, off, len, next: p[off] });
        nh = p[off];
        off += len;
    }
    Some(out)
}

/// the fragment header of a packet
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FragHdr {
    pub next: u8,
    /// the offset of the fragment's data in bytes (a multiple of 8)
    pub offset: usize,
    pub more: bool,
    pub id: u32,
}

impl FragHdr {
    pub fn read(b: &[u8]) -> FragHdr {
        let w = be16(b, 2);
        FragHdr { next: b[0], offset: (w & 0xFFF8) as usize, more: w & 1 != 0, id: be32(b, 4) }
    }
    pub fn write(&self, out: &mut [u8]) {
        out[0] = self.next;
        out[1] = 0;
        put16(out, 2, (self.offset as u16 & 0xFFF8) | self.more as u16);
        put32(out, 4, self.id);
    }
}

/// the fragment header of a packet and its offset in the payload (None: not a fragment)
pub fn frag(v: &Ipv6View) -> Option<(FragHdr, usize)> {
    let e = ext_headers(v)?.into_iter().find(|e| e.kind == NH_FRAGMENT)?;
    Some((FragHdr::read(&v.payload()[e.off..e.off + FRAG_HDR]), e.off))
}

/// the length of the unfragmentable extension headers at the start of the payload (hop-by-hop
/// options, routing headers and the destination options before a routing header)
fn unfragmentable(v: &Ipv6View) -> Option<usize> {
    let ext = ext_headers(v)?;
    let mut end = 0;
    for (i, e) in ext.iter().enumerate() {
        let before_routing = e.kind == NH_DSTOPTS && ext[i + 1..].iter().any(|x| x.kind == NH_ROUTING);
        if e.kind == NH_HOPOPT || e.kind == NH_ROUTING || before_routing {
            end = e.off + e.len;
        } else {
            break;
        }
    }
    Some(end)
}

/// splits a packet into fragments of at most `mtu` bytes (RFC 8200 4.5); a packet that fits
/// comes back alone.  None when the packet is malformed, is a fragment already, or not even
/// one 8-byte unit of data fits.
pub fn fragment(packet: &[u8], mtu: usize, id: u32) -> Option<Vec<Vec<u8>>> {
    let v = Ipv6View::parse(packet)?;
    if v.packet().len() <= mtu {
        return Some(alloc::vec![v.packet().to_vec()]);
    }
    if frag(&v).is_some() {
        return None;
    }
    let uf = unfragmentable(&v)?;
    let p = v.payload();
    // the Next Header field that announces the fragmentable part: the base header's, or that
    // of the last unfragmentable extension header
    let (next, nh_at) = match ext_headers(&v)?.into_iter().take_while(|e| e.off + e.len <= uf).last() {
        Some(e) => (e.next, IPV6_HDR + e.off),
        None => (v.next_header(), 6),
    };
    let data = &p[uf..];
    let room = mtu.checked_sub(IPV6_HDR + uf + FRAG_HDR)? & !7;
    if room == 0 {
        return None;
    }
    let mut out = Vec::new();
    let mut pos = 0;
    while pos < data.len() {
        let n = room.min(data.len() - pos);
        let mut f = Vec::with_capacity(IPV6_HDR + uf + FRAG_HDR + n);
        f.extend_from_slice(&packet[..IPV6_HDR + uf]);
        f[nh_at] = NH_FRAGMENT;
        let mut fh = [0u8; FRAG_HDR];
        FragHdr { next, offset: pos, more: pos + n < data.len(), id }.write(&mut fh);
        f.extend_from_slice(&fh);
        f.extend_from_slice(&data[pos..pos + n]);
        let pl = (f.len() - IPV6_HDR) as u16;
        put16(&mut f, 4, pl);
        out.push(f);
        pos += n;
    }
    Some(out)
}

/// lowers the hop limit by one in place; the new value
pub fn dec_hop_limit(pkt: &mut [u8]) -> u8 {
    pkt[7] = pkt[7].saturating_sub(1);
    pkt[7]
}

/// why a fragment could not be taken (the receiver answers the first two with a Parameter
/// Problem, RFC 8200 4.5)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReasmErr {
    /// a fragment with more to come whose data is not a multiple of 8 bytes (pointer: the
    /// Payload Length field)
    BadLength,
    /// offset and length go beyond 65535 bytes (pointer: the Fragment Offset field)
    TooLong,
    /// it overlaps a fragment already held: the whole packet is dropped (RFC 5722)
    Overlap,
}

/// the fragments of one packet (key: source, destination, identification)
#[derive(Clone, Debug, Default)]
pub struct Reassembly6 {
    /// the unfragmentable part of the first fragment (header and extension headers) and the
    /// Next Header of its fragment header
    first: Option<(Vec<u8>, u8)>,
    parts: BTreeMap<usize, Vec<u8>>,
    total: Option<usize>,
    bytes: usize,
    /// an overlap was seen: everything is dropped until the caller gives the packet up
    dead: bool,
}

/// the reassembly key of a fragment
pub fn reassembly_key(v: &Ipv6View, f: &FragHdr) -> (Ipv6, Ipv6, u32) {
    (v.src(), v.dst(), f.id)
}

impl Reassembly6 {
    pub fn new() -> Reassembly6 {
        Reassembly6::default()
    }
    /// adds a fragment (`at`: the offset of its fragment header in the payload); Ok(Some(the
    /// complete packet)) when all parts are there
    pub fn add(&mut self, v: &Ipv6View, f: &FragHdr, at: usize) -> Result<Option<Vec<u8>>, ReasmErr> {
        let data = v.payload().get(at + FRAG_HDR..).ok_or(ReasmErr::BadLength)?;
        if data.is_empty() || (f.more && data.len() % 8 != 0) {
            return Err(ReasmErr::BadLength);
        }
        let end = f.offset + data.len();
        if end > 65_535 {
            return Err(ReasmErr::TooLong);
        }
        if self.dead {
            return Err(ReasmErr::Overlap);
        }
        for (&o, d) in self.parts.iter() {
            if o < end && f.offset < o + d.len() {
                self.dead = true;
                self.parts.clear();
                self.first = None;
                return Err(ReasmErr::Overlap);
            }
        }
        if !f.more {
            if self.total.is_some_and(|t| t != end) {
                self.dead = true;
                return Err(ReasmErr::Overlap);
            }
            self.total = Some(end);
        }
        if self.total.is_some_and(|t| end > t || self.parts.iter().any(|(&o, d)| o + d.len() > t)) {
            self.dead = true;
            self.parts.clear();
            self.first = None;
            return Err(ReasmErr::TooLong);
        }
        if f.offset == 0 {
            self.first = Some((v.packet()[..IPV6_HDR + at].to_vec(), f.next));
        }
        self.bytes += data.len();
        self.parts.insert(f.offset, data.to_vec());
        let Some(total) = self.total else {
            return Ok(None);
        };
        if self.bytes != total {
            return Ok(None);
        }
        let Some((head, next)) = self.first.as_ref() else {
            return Ok(None);
        };
        let mut out = head.clone();
        // the Next Header that announced the fragment header now announces what it carried
        let mut nh_at = 6;
        let mut off = IPV6_HDR;
        while head[nh_at] != NH_FRAGMENT {
            if !matches!(head[nh_at], NH_HOPOPT | NH_ROUTING | NH_DSTOPTS) || off + 2 > head.len() {
                return Err(ReasmErr::Overlap);
            }
            nh_at = off;
            off += (head[off + 1] as usize + 1) * 8;
        }
        out[nh_at] = *next;
        for d in self.parts.values() {
            out.extend_from_slice(d);
        }
        let pl = u16::try_from(out.len() - IPV6_HDR).map_err(|_| ReasmErr::TooLong)?;
        put16(&mut out, 4, pl);
        Ok(Some(out))
    }
    /// the first fragment (offset 0) is here: a timeout is answered with Time Exceeded
    pub fn has_first(&self) -> bool {
        self.first.is_some()
    }
    /// the source and header of the first fragment (what a timeout's error quotes)
    pub fn first_packet(&self) -> Option<Vec<u8>> {
        let (h, _) = self.first.as_ref()?;
        let mut p = h.clone();
        if let Some(d) = self.parts.get(&0) {
            p.extend_from_slice(d);
        }
        Some(p)
    }
    /// bytes collected so far
    pub fn collected(&self) -> usize {
        self.bytes
    }
}

/// an IPv6 fixed header to write
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ipv6Hdr {
    pub traffic_class: u8,
    pub flow_label: u32,
    pub next_header: u8,
    pub hop_limit: u8,
    pub src: Ipv6,
    pub dst: Ipv6,
}

impl Ipv6Hdr {
    pub const LEN: usize = IPV6_HDR;

    pub fn new(src: Ipv6, dst: Ipv6, next_header: u8, hop_limit: u8) -> Ipv6Hdr {
        Ipv6Hdr { traffic_class: 0, flow_label: 0, next_header, hop_limit, src, dst }
    }
    pub fn read(v: &Ipv6View) -> Ipv6Hdr {
        Ipv6Hdr {
            traffic_class: v.traffic_class(),
            flow_label: v.flow_label(),
            next_header: v.next_header(),
            hop_limit: v.hop_limit(),
            src: v.src(),
            dst: v.dst(),
        }
    }
    pub fn write(&self, out: &mut [u8], payload_len: usize) {
        let w = (6u32 << 28) | ((self.traffic_class as u32) << 20) | (self.flow_label & 0xF_FFFF);
        out[0..4].copy_from_slice(&w.to_be_bytes());
        put16(out, 4, payload_len as u16);
        out[6] = self.next_header;
        out[7] = self.hop_limit;
        out[8..24].copy_from_slice(&self.src.0);
        out[24..40].copy_from_slice(&self.dst.0);
    }
    pub fn push_onto(&self, p: &mut Pkt) {
        let pl = p.len();
        self.write(p.push(IPV6_HDR), pl);
    }
    /// header + payload as a new packet
    pub fn packet(&self, payload: &[u8]) -> Vec<u8> {
        let mut v = alloc::vec![0u8; IPV6_HDR + payload.len()];
        self.write(&mut v, payload.len());
        v[IPV6_HDR..].copy_from_slice(payload);
        v
    }
}
