// nat -- address and port translation of IPv4 packets in place (RFC 3022): addresses, TCP and
// UDP ports and the identifiers of ICMP queries change, and every checksum that covers them
// follows incrementally (RFC 1624); ICMP errors take the packet they quote along (RFC 5508)
//
//   Flow::of(packet) -> Option<Flow>      protocol, addresses, ports (the identifier of an ICMP
//                                         query in both), ICMP type and code, TCP flags; a
//                                         fragment after the first has no ports (`ports`
//                                         false)
//   Flow::quoted(packet) -> Option<Flow>  the packet an ICMP error quotes (its header and at
//                                         least 8 bytes of its payload)
//   rewrite(pkt, side, addr, port)        the source or destination address of a packet and,
//                                         with Some, its port (an ICMP query: its identifier);
//                                         the IPv4 header checksum, the TCP or UDP checksum
//                                         (pseudo header and port; a UDP checksum of 0 means
//                                         none and stays 0) and the ICMP checksum follow; a
//                                         later fragment changes its address only
//   rewrite_quoted(pkt, side, addr, port) the same for the packet inside an ICMP error: its
//                                         header checksum, its TCP or UDP checksum when the
//                                         quote holds it, an ICMP query's checksum, and the
//                                         error's own ICMP checksum over every changed byte
//                                         (the outer header stays; rewrite() changes it)
//   is_query(type) / reply_of(type)       ICMP queries that carry an identifier (echo,
//                                         timestamp, information, address mask) and the type
//                                         of their answers
//
// Translation needs no full recomputation: the segment of a first fragment is not complete,
// yet its checksum covers all of it, and an ICMP error quotes only the start of a packet.
// Every field these functions change lies on a 16-bit boundary of the data its checksum
// covers, so the old and new words are enough (checksum::update).

use crate::addr::Ipv4;
use crate::checksum;
use crate::icmp::{self, IcmpView};
use crate::ipv4::{Ipv4View, PROTO_ICMP, PROTO_TCP, PROTO_UDP};
use crate::{be16, put16};

/// which address and port of a packet a translation changes
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Src,
    Dst,
}

/// what translation and filters look at in a packet
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Flow {
    pub proto: u8,
    pub src: Ipv4,
    pub dst: Ipv4,
    /// TCP and UDP ports; an ICMP query's identifier in both; 0 when unknown
    pub sport: u16,
    pub dport: u16,
    /// the ports (or the identifier) are known: not a later fragment, and the transport header
    /// is there
    pub ports: bool,
    /// ICMP type and code
    pub icmp: Option<(u8, u8)>,
    /// TCP flags (netpkt::tcp::SYN ...)
    pub flags: u8,
    /// a fragment after the first (no transport header)
    pub later: bool,
    /// more fragments follow
    pub mf: bool,
    /// the identification field (fragments of one datagram share it)
    pub id: u16,
}

/// ICMP queries with an identifier: echo, timestamp, information, address mask (requests and
/// replies)
pub fn is_query(typ: u8) -> bool {
    matches!(typ, 0 | 8 | 13 | 14 | 15 | 16 | 17 | 18)
}

/// the type of the answer to an ICMP query request (None: not a request)
pub fn reply_of(typ: u8) -> Option<u8> {
    match typ {
        icmp::T_ECHO_REQUEST => Some(icmp::T_ECHO_REPLY),
        13 => Some(14),
        15 => Some(16),
        17 => Some(18),
        _ => None,
    }
}

impl Flow {
    /// the flow of a whole packet
    pub fn of(packet: &[u8]) -> Option<Flow> {
        Flow::read(&Ipv4View::parse(packet)?)
    }
    /// the flow of the packet an ICMP error quotes (`packet`: the error's whole IPv4 packet)
    pub fn quoted(packet: &[u8]) -> Option<Flow> {
        let v = Ipv4View::parse(packet)?;
        let ic = IcmpView::parse(v.payload())?;
        Flow::read(&ic.quoted()?)
    }
    fn read(v: &Ipv4View) -> Option<Flow> {
        let later = v.frag_off() > 0;
        let mut f = Flow { proto: v.proto(), src: v.src(), dst: v.dst(), sport: 0, dport: 0, ports: false, icmp: None, flags: 0, later, mf: v.mf(), id: v.id() };
        if later {
            return Some(f);
        }
        let p = v.payload();
        match f.proto {
            PROTO_TCP | PROTO_UDP if p.len() >= 4 => {
                f.sport = be16(p, 0);
                f.dport = be16(p, 2);
                f.ports = true;
                if f.proto == PROTO_TCP && p.len() >= 14 {
                    f.flags = p[13];
                }
            }
            PROTO_ICMP if p.len() >= 2 => {
                f.icmp = Some((p[0], p[1]));
                if is_query(p[0]) && p.len() >= 6 {
                    f.sport = be16(p, 4);
                    f.dport = f.sport;
                    f.ports = true;
                }
            }
            _ => {}
        }
        Some(f)
    }
}

/// writes `new` (at most 4 bytes) at `at` and updates the checksum at `ck` for it; `at` lies on
/// the word grid of the data that checksum covers
fn patch(b: &mut [u8], at: usize, new: &[u8], ck: usize) {
    let old: [u8; 4] = {
        let mut o = [0u8; 4];
        o[..new.len()].copy_from_slice(&b[at..at + new.len()]);
        o
    };
    let c = checksum::update(be16(b, ck), &old[..new.len()], new);
    b[at..at + new.len()].copy_from_slice(new);
    put16(b, ck, c);
}

/// the transport part of a translation inside one packet (`l4`: where its header starts, `len`:
/// how many of its bytes are there): the port of `side` (or a query's identifier) and the
/// checksum, which also covers the pseudo header's `old` / `new` address
fn transport(b: &mut [u8], proto: u8, l4: usize, len: usize, side: Side, old: [u8; 4], new: [u8; 4], port: Option<u16>) {
    let poff = if side == Side::Src { 0 } else { 2 };
    match proto {
        PROTO_TCP | PROTO_UDP => {
            let (ckoff, udp) = if proto == PROTO_TCP { (16, false) } else { (6, true) };
            let have_ck = len >= ckoff + 2;
            // a UDP checksum of 0 means "none" (RFC 768) and stays so
            let live = have_ck && !(udp && be16(b, l4 + ckoff) == 0);
            let oldp = if len >= poff + 2 { Some(be16(b, l4 + poff)) } else { None };
            if live {
                let mut c = checksum::update(be16(b, l4 + ckoff), &old, &new);
                if let (Some(p), Some(o)) = (port, oldp) {
                    c = checksum::update16(c, o, p);
                }
                if udp && c == 0 {
                    c = 0xFFFF;
                }
                put16(b, l4 + ckoff, c);
            }
            if let (Some(p), Some(_)) = (port, oldp) {
                put16(b, l4 + poff, p);
            }
        }
        PROTO_ICMP => {
            // a query's identifier: the ICMP checksum has no pseudo header
            if let Some(p) = port
                && len >= 6
                && is_query(b[l4])
            {
                let o = be16(b, l4 + 4);
                let c = checksum::update16(be16(b, l4 + 2), o, p);
                put16(b, l4 + 2, c);
                put16(b, l4 + 4, p);
            }
        }
        _ => {}
    }
}

/// sets the source or destination address of an IPv4 packet and, with `port`, its TCP / UDP
/// port or ICMP query identifier, with every checksum updated; false when it is no IPv4 packet
pub fn rewrite(pkt: &mut [u8], side: Side, addr: Ipv4, port: Option<u16>) -> bool {
    let Some(v) = Ipv4View::parse_truncated(pkt) else {
        return false;
    };
    let (hl, proto, later, end) = (v.header_len(), v.proto(), v.frag_off() > 0, v.packet().len());
    let at = if side == Side::Src { 12 } else { 16 };
    let old: [u8; 4] = [pkt[at], pkt[at + 1], pkt[at + 2], pkt[at + 3]];
    patch(pkt, at, &addr.0, 10);
    if !later {
        transport(pkt, proto, hl, end - hl, side, old, addr.0, port);
    }
    true
}

/// sets the source or destination address (and port or query identifier) of the packet an
/// ICMP error quotes, with the quote's checksums and the error's ICMP checksum; false when
/// `pkt` is no ICMP error with a quoted IPv4 header
pub fn rewrite_quoted(pkt: &mut [u8], side: Side, addr: Ipv4, port: Option<u16>) -> bool {
    let Some(v) = Ipv4View::parse(pkt) else {
        return false;
    };
    let (ohl, end) = (v.header_len(), v.packet().len());
    let Some(ic) = IcmpView::parse(v.payload()) else {
        return false;
    };
    let Some(q) = ic.quoted() else {
        return false;
    };
    let (qhl, qproto, qlater) = (q.header_len(), q.proto(), q.frag_off() > 0);
    let qs = ohl + icmp::ICMP_HDR;
    // the ICMP message before the change: its checksum follows every changed word
    let before: alloc::vec::Vec<u8> = pkt[ohl..end].to_vec();
    let at = qs + if side == Side::Src { 12 } else { 16 };
    let old: [u8; 4] = [pkt[at], pkt[at + 1], pkt[at + 2], pkt[at + 3]];
    patch(pkt, at, &addr.0, qs + 10);
    if !qlater {
        let l4 = qs + qhl;
        transport(pkt, qproto, l4, end.saturating_sub(l4), side, old, addr.0, port);
    }
    let c = checksum::update(be16(pkt, ohl + 2), &before, &pkt[ohl..end]);
    put16(pkt, ohl + 2, c);
    true
}
