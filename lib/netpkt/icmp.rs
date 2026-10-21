// icmp -- ICMP for IPv4 (RFC 792, 1191, 1812)
//
//   IcmpView::parse(b)    type, code, checksum, the 4 bytes after it, payload
//   Icmp                  EchoRequest / EchoReply { id, seq }, Unreachable { code, mtu },
//                         TimeExceeded { code }, Redirect { code, gateway }, ParamProblem,
//                         Other; encode(payload) with checksum, push_onto(&mut Pkt)
//   error_quote(packet)   what an error message quotes of the offending packet: its header and
//                         the first 8 bytes of its payload
//   name(type, code)      "Echo (ping) request", "Destination unreachable (Port unreachable)", ...

use alloc::vec::Vec;

use crate::addr::Ipv4;
use crate::checksum;
use crate::ipv4::Ipv4View;
use crate::pkt::Pkt;
use crate::{be16, put16};

pub const ICMP_HDR: usize = 8;

pub const T_ECHO_REPLY: u8 = 0;
pub const T_UNREACH: u8 = 3;
pub const T_REDIRECT: u8 = 5;
pub const T_ECHO_REQUEST: u8 = 8;
pub const T_TIME_EXCEEDED: u8 = 11;
pub const T_PARAM_PROBLEM: u8 = 12;

pub const UNREACH_NET: u8 = 0;
pub const UNREACH_HOST: u8 = 1;
pub const UNREACH_PROTO: u8 = 2;
pub const UNREACH_PORT: u8 = 3;
pub const UNREACH_FRAG_NEEDED: u8 = 4;
pub const UNREACH_ADMIN: u8 = 13;

pub const EXCEEDED_TTL: u8 = 0;
pub const EXCEEDED_REASSEMBLY: u8 = 1;

/// a received ICMP message
#[derive(Clone, Copy, Debug)]
pub struct IcmpView<'a> {
    b: &'a [u8],
}

impl<'a> IcmpView<'a> {
    pub fn parse(b: &'a [u8]) -> Option<IcmpView<'a>> {
        if b.len() < ICMP_HDR { None } else { Some(IcmpView { b }) }
    }
    pub fn typ(&self) -> u8 {
        self.b[0]
    }
    pub fn code(&self) -> u8 {
        self.b[1]
    }
    pub fn checksum(&self) -> u16 {
        be16(self.b, 2)
    }
    pub fn checksum_ok(&self) -> bool {
        checksum::verify(self.b)
    }
    /// the 4 bytes after the checksum
    pub fn rest(&self) -> [u8; 4] {
        [self.b[4], self.b[5], self.b[6], self.b[7]]
    }
    pub fn id(&self) -> u16 {
        be16(self.b, 4)
    }
    pub fn seq(&self) -> u16 {
        be16(self.b, 6)
    }
    pub fn payload(&self) -> &'a [u8] {
        &self.b[ICMP_HDR..]
    }
    /// the quoted packet of an error message (unreachable, time exceeded, redirect, ...)
    pub fn quoted(&self) -> Option<Ipv4View<'a>> {
        match self.typ() {
            T_UNREACH | T_REDIRECT | T_TIME_EXCEEDED | T_PARAM_PROBLEM => Ipv4View::parse_truncated(self.payload()),
            _ => None,
        }
    }
    pub fn is_error(&self) -> bool {
        matches!(self.typ(), T_UNREACH | T_REDIRECT | T_TIME_EXCEEDED | T_PARAM_PROBLEM)
    }
}

/// the header and the first 8 payload bytes of the offending packet (what an ICMP error quotes)
pub fn error_quote(packet: &[u8]) -> &[u8] {
    match Ipv4View::parse(packet) {
        Some(v) => &packet[..(v.header_len() + 8).min(v.total_len())],
        None => &packet[..packet.len().min(28)],
    }
}

/// an ICMP message to write
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icmp {
    EchoRequest { id: u16, seq: u16 },
    EchoReply { id: u16, seq: u16 },
    /// destination unreachable; `mtu` is the next-hop MTU of code 4 (RFC 1191), else 0
    Unreachable { code: u8, mtu: u16 },
    TimeExceeded { code: u8 },
    Redirect { code: u8, gateway: Ipv4 },
    ParamProblem { pointer: u8 },
    Other { typ: u8, code: u8, rest: [u8; 4] },
}

impl Icmp {
    pub fn read(v: &IcmpView) -> Icmp {
        let r = v.rest();
        match v.typ() {
            T_ECHO_REQUEST => Icmp::EchoRequest { id: v.id(), seq: v.seq() },
            T_ECHO_REPLY => Icmp::EchoReply { id: v.id(), seq: v.seq() },
            T_UNREACH => Icmp::Unreachable { code: v.code(), mtu: u16::from_be_bytes([r[2], r[3]]) },
            T_TIME_EXCEEDED => Icmp::TimeExceeded { code: v.code() },
            T_REDIRECT => Icmp::Redirect { code: v.code(), gateway: Ipv4(r) },
            T_PARAM_PROBLEM => Icmp::ParamProblem { pointer: r[0] },
            t => Icmp::Other { typ: t, code: v.code(), rest: r },
        }
    }
    pub fn type_code(&self) -> (u8, u8) {
        match *self {
            Icmp::EchoRequest { .. } => (T_ECHO_REQUEST, 0),
            Icmp::EchoReply { .. } => (T_ECHO_REPLY, 0),
            Icmp::Unreachable { code, .. } => (T_UNREACH, code),
            Icmp::TimeExceeded { code } => (T_TIME_EXCEEDED, code),
            Icmp::Redirect { code, .. } => (T_REDIRECT, code),
            Icmp::ParamProblem { .. } => (T_PARAM_PROBLEM, 0),
            Icmp::Other { typ, code, .. } => (typ, code),
        }
    }
    fn rest(&self) -> [u8; 4] {
        match *self {
            Icmp::EchoRequest { id, seq } | Icmp::EchoReply { id, seq } => {
                let (a, b) = (id.to_be_bytes(), seq.to_be_bytes());
                [a[0], a[1], b[0], b[1]]
            }
            Icmp::Unreachable { mtu, .. } => {
                let m = mtu.to_be_bytes();
                [0, 0, m[0], m[1]]
            }
            Icmp::Redirect { gateway, .. } => gateway.0,
            Icmp::ParamProblem { pointer } => [pointer, 0, 0, 0],
            Icmp::TimeExceeded { .. } => [0; 4],
            Icmp::Other { rest, .. } => rest,
        }
    }
    /// header + payload with the checksum
    pub fn encode(&self, payload: &[u8]) -> Vec<u8> {
        let mut v = alloc::vec![0u8; ICMP_HDR + payload.len()];
        let (t, c) = self.type_code();
        v[0] = t;
        v[1] = c;
        v[4..8].copy_from_slice(&self.rest());
        v[ICMP_HDR..].copy_from_slice(payload);
        let ck = checksum::checksum(&v);
        put16(&mut v, 2, ck);
        v
    }
    /// prepends the header; the packet's current content is the payload
    pub fn push_onto(&self, p: &mut Pkt) {
        let (t, c) = self.type_code();
        let rest = self.rest();
        let h = p.push(ICMP_HDR);
        h[0] = t;
        h[1] = c;
        h[4..8].copy_from_slice(&rest);
        let ck = checksum::checksum(p.bytes());
        put16(p.bytes_mut(), 2, ck);
    }
}

/// a readable name of an ICMP type and code
pub fn name(typ: u8, code: u8) -> &'static str {
    match (typ, code) {
        (T_ECHO_REPLY, _) => "Echo (ping) reply",
        (T_ECHO_REQUEST, _) => "Echo (ping) request",
        (T_UNREACH, UNREACH_NET) => "Destination unreachable (Network unreachable)",
        (T_UNREACH, UNREACH_HOST) => "Destination unreachable (Host unreachable)",
        (T_UNREACH, UNREACH_PROTO) => "Destination unreachable (Protocol unreachable)",
        (T_UNREACH, UNREACH_PORT) => "Destination unreachable (Port unreachable)",
        (T_UNREACH, UNREACH_FRAG_NEEDED) => "Destination unreachable (Fragmentation needed)",
        (T_UNREACH, UNREACH_ADMIN) => "Destination unreachable (Administratively prohibited)",
        (T_UNREACH, _) => "Destination unreachable",
        (T_REDIRECT, _) => "Redirect",
        (T_TIME_EXCEEDED, EXCEEDED_TTL) => "Time-to-live exceeded",
        (T_TIME_EXCEEDED, _) => "Fragment reassembly time exceeded",
        (T_PARAM_PROBLEM, _) => "Parameter problem",
        (13, _) => "Timestamp request",
        (14, _) => "Timestamp reply",
        _ => "ICMP",
    }
}
