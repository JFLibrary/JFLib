// ipv6 -- the IPv6 fixed header (RFC 8200) and the walk over extension headers
//
//   Ipv6View::parse(b)    version 6, payload length within the buffer
//   Ipv6Hdr { .. }        write(out, payload_len), push_onto(&mut Pkt)
//   upper_layer(v)        (next header, offset in the payload) after hop-by-hop, routing,
//                         fragment and destination options headers
//
// ICMPv6 / NDP and SLAAC follow with the IPv6 milestone of the simulator.

use alloc::vec::Vec;

use crate::addr::Ipv6;
use crate::pkt::Pkt;
use crate::{be16, put16};

pub const IPV6_HDR: usize = 40;

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

/// a received IPv6 packet
#[derive(Clone, Copy, Debug)]
pub struct Ipv6View<'a> {
    b: &'a [u8],
}

impl<'a> Ipv6View<'a> {
    pub fn parse(b: &'a [u8]) -> Option<Ipv6View<'a>> {
        if b.len() < IPV6_HDR || b[0] >> 4 != 6 || IPV6_HDR + be16(b, 4) as usize > b.len() {
            return None;
        }
        Some(Ipv6View { b })
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
    pub fn payload(&self) -> &'a [u8] {
        &self.b[IPV6_HDR..IPV6_HDR + self.payload_len()]
    }
    pub fn packet(&self) -> &'a [u8] {
        &self.b[..IPV6_HDR + self.payload_len()]
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
