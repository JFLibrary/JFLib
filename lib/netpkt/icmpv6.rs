// icmpv6 -- ICMP for IPv6 (RFC 4443) and the messages of Neighbor Discovery (RFC 4861)
//
//   Icmp6View::parse(b)     type, code, checksum (checksum_ok over the IPv6 pseudo header, RFC
//                           4443 2.3), the 4 bytes after it, the body; quoted() for the packet
//                           an error message carries
//   Icmp6                   EchoRequest / EchoReply { id, seq }, DestUnreach { code },
//                           PacketTooBig { mtu }, TimeExceeded { code }, ParamProblem { code,
//                           pointer }, Other; encode(src, dst, body) and push_v6(&mut Pkt, src,
//                           dst) with the checksum
//   Nd { msg, opts }        RouterSolicit, RouterAdvert(Ra), NeighborSolicit { target },
//                           NeighborAdvert { router, solicited, override_, target },
//                           Redirect { target, dst } with their options (NdOpt: source / target
//                           link-layer address, prefix information, redirected header, MTU, any
//                           other kept as bytes); Nd::parse checks what RFC 4861 asks a receiver
//                           to check in the message itself (code 0, lengths, no option of length
//                           0, no group as target); hop limit 255, the checksum and the rules
//                           that need the addresses are the caller's (check_ip)
//   Nd::encode(src, dst)    the message with its checksum
//   error_quote(packet)     what an error quotes: as much of the offending packet as fits into
//                           an ICMPv6 packet of the minimum MTU (1280 bytes, RFC 4443 2.4 (c))
//   name(type, code)        "Echo (ping) request", "Neighbor Solicitation", "Time Exceeded (hop
//                           limit exceeded in transit)", ... (the wording of capture tools)
//
// The multicast addresses and MACs of ND are Ipv6::solicited_node, Ipv6::ALL_NODES,
// ALL_ROUTERS and Mac::of_ipv6_multicast (addr.rs); interface identifiers Mac::eui64.

use alloc::vec::Vec;

use crate::addr::{Ipv6, Mac};
use crate::checksum;
use crate::ipv6::{Ipv6View, IPV6_HDR, MIN_MTU, NH_ICMPV6};
use crate::pkt::Pkt;
use crate::{be16, be32, put16};

/// type, code, checksum and the 4 bytes every message has after them
pub const ICMPV6_HDR: usize = 8;

pub const T_DEST_UNREACH: u8 = 1;
pub const T_PACKET_TOO_BIG: u8 = 2;
pub const T_TIME_EXCEEDED: u8 = 3;
pub const T_PARAM_PROBLEM: u8 = 4;
pub const T_ECHO_REQUEST: u8 = 128;
pub const T_ECHO_REPLY: u8 = 129;
pub const T_MLD_QUERY: u8 = 130;
pub const T_MLD_REPORT: u8 = 131;
pub const T_MLD_DONE: u8 = 132;
pub const T_ROUTER_SOLICIT: u8 = 133;
pub const T_ROUTER_ADVERT: u8 = 134;
pub const T_NEIGHBOR_SOLICIT: u8 = 135;
pub const T_NEIGHBOR_ADVERT: u8 = 136;
pub const T_REDIRECT: u8 = 137;
pub const T_MLD2_REPORT: u8 = 143;

/// codes of Destination Unreachable
pub const UNREACH_NO_ROUTE: u8 = 0;
pub const UNREACH_ADMIN: u8 = 1;
pub const UNREACH_BEYOND_SCOPE: u8 = 2;
pub const UNREACH_ADDR: u8 = 3;
pub const UNREACH_PORT: u8 = 4;
pub const UNREACH_POLICY: u8 = 5;
pub const UNREACH_REJECT: u8 = 6;
/// codes of Time Exceeded
pub const EXCEEDED_HOP_LIMIT: u8 = 0;
pub const EXCEEDED_REASSEMBLY: u8 = 1;
/// codes of Parameter Problem
pub const PARAM_HEADER: u8 = 0;
pub const PARAM_NEXT_HEADER: u8 = 1;
pub const PARAM_OPTION: u8 = 2;

/// ND option types (RFC 4861 4.6)
pub const OPT_SRC_LLA: u8 = 1;
pub const OPT_TGT_LLA: u8 = 2;
pub const OPT_PREFIX: u8 = 3;
pub const OPT_REDIRECTED: u8 = 4;
pub const OPT_MTU: u8 = 5;

/// the lifetime that never ends (prefix information, RFC 4861 4.6.2)
pub const INFINITE: u32 = 0xFFFF_FFFF;

/// the fixed part of each ND message before its options
const RS_LEN: usize = 8;
const RA_LEN: usize = 16;
const NS_LEN: usize = 24;
const NA_LEN: usize = 24;
const REDIRECT_LEN: usize = 40;

/// a received ICMPv6 message
#[derive(Clone, Copy, Debug)]
pub struct Icmp6View<'a> {
    b: &'a [u8],
}

impl<'a> Icmp6View<'a> {
    /// at least type, code and checksum
    pub fn parse(b: &'a [u8]) -> Option<Icmp6View<'a>> {
        if b.len() < 4 { None } else { Some(Icmp6View { b }) }
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
    /// the checksum over the pseudo header of `src` and `dst` and the whole message
    pub fn checksum_ok(&self, src: Ipv6, dst: Ipv6) -> bool {
        checksum::fold(checksum::sum(checksum::pseudo_v6(src, dst, NH_ICMPV6, self.b.len() as u32), self.b)) == 0xFFFF
    }
    /// the checksum the message should carry (for a dissector's "should be")
    pub fn checksum_want(&self, src: Ipv6, dst: Ipv6) -> u16 {
        let mut c = self.b.to_vec();
        c[2] = 0;
        c[3] = 0;
        checksum::finish(checksum::sum(checksum::pseudo_v6(src, dst, NH_ICMPV6, c.len() as u32), &c))
    }
    /// the 4 bytes after the checksum (zeros when the message is shorter)
    pub fn rest(&self) -> [u8; 4] {
        let mut r = [0u8; 4];
        for (i, x) in r.iter_mut().enumerate() {
            *x = self.b.get(4 + i).copied().unwrap_or(0);
        }
        r
    }
    pub fn id(&self) -> u16 {
        u16::from_be_bytes([self.rest()[0], self.rest()[1]])
    }
    pub fn seq(&self) -> u16 {
        u16::from_be_bytes([self.rest()[2], self.rest()[3]])
    }
    /// the MTU of Packet Too Big, the pointer of Parameter Problem
    pub fn value(&self) -> u32 {
        u32::from_be_bytes(self.rest())
    }
    /// what follows the first 8 bytes (an error's quote, an echo's data)
    pub fn body(&self) -> &'a [u8] {
        if self.b.len() > ICMPV6_HDR { &self.b[ICMPV6_HDR..] } else { &[] }
    }
    pub fn bytes(&self) -> &'a [u8] {
        self.b
    }
    pub fn len(&self) -> usize {
        self.b.len()
    }
    pub fn is_empty(&self) -> bool {
        self.b.is_empty()
    }
    /// an error message (types 0 to 127, RFC 4443 2.1)
    pub fn is_error(&self) -> bool {
        self.typ() < 128
    }
    /// a message of Neighbor Discovery
    pub fn is_nd(&self) -> bool {
        (T_ROUTER_SOLICIT..=T_REDIRECT).contains(&self.typ())
    }
    /// the packet an error message quotes (cut where the quote ends)
    pub fn quoted(&self) -> Option<Ipv6View<'a>> {
        match self.typ() {
            T_DEST_UNREACH | T_PACKET_TOO_BIG | T_TIME_EXCEEDED | T_PARAM_PROBLEM => Ipv6View::parse_truncated(self.body()),
            _ => None,
        }
    }
}

/// as much of the offending packet as an error may quote: the error packet stays within the
/// minimum MTU of 1280 bytes (40 bytes IPv6, 8 bytes ICMPv6, at most 1232 of the packet)
pub fn error_quote(packet: &[u8]) -> &[u8] {
    &packet[..packet.len().min(MIN_MTU - IPV6_HDR - ICMPV6_HDR)]
}

/// an ICMPv6 message with the 8-byte header (echo and the errors) to write
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icmp6 {
    EchoRequest { id: u16, seq: u16 },
    EchoReply { id: u16, seq: u16 },
    DestUnreach { code: u8 },
    PacketTooBig { mtu: u32 },
    TimeExceeded { code: u8 },
    ParamProblem { code: u8, pointer: u32 },
    Other { typ: u8, code: u8, rest: [u8; 4] },
}

impl Icmp6 {
    pub fn read(v: &Icmp6View) -> Icmp6 {
        match v.typ() {
            T_ECHO_REQUEST => Icmp6::EchoRequest { id: v.id(), seq: v.seq() },
            T_ECHO_REPLY => Icmp6::EchoReply { id: v.id(), seq: v.seq() },
            T_DEST_UNREACH => Icmp6::DestUnreach { code: v.code() },
            T_PACKET_TOO_BIG => Icmp6::PacketTooBig { mtu: v.value() },
            T_TIME_EXCEEDED => Icmp6::TimeExceeded { code: v.code() },
            T_PARAM_PROBLEM => Icmp6::ParamProblem { code: v.code(), pointer: v.value() },
            t => Icmp6::Other { typ: t, code: v.code(), rest: v.rest() },
        }
    }
    pub fn type_code(&self) -> (u8, u8) {
        match *self {
            Icmp6::EchoRequest { .. } => (T_ECHO_REQUEST, 0),
            Icmp6::EchoReply { .. } => (T_ECHO_REPLY, 0),
            Icmp6::DestUnreach { code } => (T_DEST_UNREACH, code),
            Icmp6::PacketTooBig { .. } => (T_PACKET_TOO_BIG, 0),
            Icmp6::TimeExceeded { code } => (T_TIME_EXCEEDED, code),
            Icmp6::ParamProblem { code, .. } => (T_PARAM_PROBLEM, code),
            Icmp6::Other { typ, code, .. } => (typ, code),
        }
    }
    fn rest(&self) -> [u8; 4] {
        match *self {
            Icmp6::EchoRequest { id, seq } | Icmp6::EchoReply { id, seq } => {
                let (a, b) = (id.to_be_bytes(), seq.to_be_bytes());
                [a[0], a[1], b[0], b[1]]
            }
            Icmp6::PacketTooBig { mtu } => mtu.to_be_bytes(),
            Icmp6::ParamProblem { pointer, .. } => pointer.to_be_bytes(),
            Icmp6::DestUnreach { .. } | Icmp6::TimeExceeded { .. } => [0; 4],
            Icmp6::Other { rest, .. } => rest,
        }
    }
    pub fn is_error(&self) -> bool {
        self.type_code().0 < 128
    }
    /// header + body with the checksum over the pseudo header
    pub fn encode(&self, src: Ipv6, dst: Ipv6, body: &[u8]) -> Vec<u8> {
        let mut v = alloc::vec![0u8; ICMPV6_HDR + body.len()];
        let (t, c) = self.type_code();
        v[0] = t;
        v[1] = c;
        v[4..8].copy_from_slice(&self.rest());
        v[ICMPV6_HDR..].copy_from_slice(body);
        seal(&mut v, src, dst);
        v
    }
    /// prepends the header (the packet's content is the body) with the checksum
    pub fn push_v6(&self, p: &mut Pkt, src: Ipv6, dst: Ipv6) {
        let (t, c) = self.type_code();
        let rest = self.rest();
        let h = p.push(ICMPV6_HDR);
        h[0] = t;
        h[1] = c;
        h[4..8].copy_from_slice(&rest);
        seal(p.bytes_mut(), src, dst);
    }
}

/// writes the checksum of a whole ICMPv6 message (its checksum field is 0)
fn seal(m: &mut [u8], src: Ipv6, dst: Ipv6) {
    let ck = checksum::finish(checksum::sum(checksum::pseudo_v6(src, dst, NH_ICMPV6, m.len() as u32), m));
    put16(m, 2, ck);
}

// ---------------------------------------------------------------- Neighbor Discovery

/// the fixed fields of a Router Advertisement (RFC 4861 4.2; the preference bits of RFC 4191)
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Ra {
    /// 0: unspecified
    pub hop_limit: u8,
    /// addresses from DHCPv6 (M) / other settings from DHCPv6 (O)
    pub managed: bool,
    pub other: bool,
    /// default router preference: 0 medium, 1 high, 3 low (2 is reserved, read as medium)
    pub prf: u8,
    /// seconds; 0: not a default router
    pub lifetime: u16,
    /// milliseconds; 0: unspecified
    pub reachable: u32,
    pub retrans: u32,
}

/// a prefix information option (RFC 4861 4.6.2)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PrefixInfo {
    pub len: u8,
    /// the prefix is on the link (L)
    pub on_link: bool,
    /// addresses may be formed from it (A, RFC 4862)
    pub auto: bool,
    /// seconds, INFINITE for ever
    pub valid: u32,
    pub preferred: u32,
    pub prefix: Ipv6,
}

/// an option of an ND message
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NdOpt {
    SrcLla(Mac),
    TgtLla(Mac),
    Prefix(PrefixInfo),
    /// what a redirect carries of the redirected packet
    Redirected(Vec<u8>),
    Mtu(u32),
    /// any other option: its type and its bytes after type and length
    Other(u8, Vec<u8>),
}

impl NdOpt {
    pub fn kind(&self) -> u8 {
        match self {
            NdOpt::SrcLla(_) => OPT_SRC_LLA,
            NdOpt::TgtLla(_) => OPT_TGT_LLA,
            NdOpt::Prefix(_) => OPT_PREFIX,
            NdOpt::Redirected(_) => OPT_REDIRECTED,
            NdOpt::Mtu(_) => OPT_MTU,
            NdOpt::Other(t, _) => *t,
        }
    }
    /// appends type, length (in units of 8 bytes) and value, padded to a multiple of 8
    fn write(&self, out: &mut Vec<u8>) {
        let at = out.len();
        out.push(self.kind());
        out.push(0);
        match self {
            NdOpt::SrcLla(m) | NdOpt::TgtLla(m) => out.extend_from_slice(&m.0),
            NdOpt::Prefix(p) => {
                out.push(p.len);
                out.push(if p.on_link { 0x80 } else { 0 } | if p.auto { 0x40 } else { 0 });
                out.extend_from_slice(&p.valid.to_be_bytes());
                out.extend_from_slice(&p.preferred.to_be_bytes());
                out.extend_from_slice(&[0; 4]);
                out.extend_from_slice(&p.prefix.0);
            }
            NdOpt::Redirected(d) => {
                out.extend_from_slice(&[0; 6]);
                // the length field is one byte of 8-byte units
                out.extend_from_slice(&d[..d.len().min(255 * 8 - 8)]);
            }
            NdOpt::Mtu(m) => {
                out.extend_from_slice(&[0; 2]);
                out.extend_from_slice(&m.to_be_bytes());
            }
            NdOpt::Other(_, d) => out.extend_from_slice(&d[..d.len().min(255 * 8 - 2)]),
        }
        while (out.len() - at) % 8 != 0 {
            out.push(0);
        }
        out[at + 1] = ((out.len() - at) / 8) as u8;
    }
}

/// the message of ND
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NdMsg {
    RouterSolicit,
    RouterAdvert(Ra),
    NeighborSolicit { target: Ipv6 },
    NeighborAdvert { router: bool, solicited: bool, override_: bool, target: Ipv6 },
    Redirect { target: Ipv6, dst: Ipv6 },
}

/// why a receiver drops an ND message (RFC 4861 6.1, 7.1, 8.1)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NdErr {
    /// the ICMP code is not 0
    Code(u8),
    /// shorter than the message's fixed part
    Short,
    /// an option with length 0, or one that runs past the end
    BadOption,
    /// a group address as the target of NS / NA or as the destination of a redirect
    Multicast,
    /// not an ND type
    NotNd,
    /// the hop limit is not 255: the message did not come from this link
    HopLimit,
    /// a router advertisement or redirect from an address that is not link-local
    Source,
    /// an address detection probe (unspecified source) not sent to a solicited-node group
    DadDest,
    /// an unspecified source with a source link-layer address option
    SourceLla,
    /// a solicited advertisement sent to a group
    SolicitedToGroup,
}

impl NdErr {
    /// the reason in the words of capture tools (English)
    pub fn text(&self) -> &'static str {
        match self {
            NdErr::Code(_) => "the code is not 0",
            NdErr::Short => "shorter than its fixed fields",
            NdErr::BadOption => "an option with length 0 or past the end",
            NdErr::Multicast => "a multicast address as the target",
            NdErr::NotNd => "not a Neighbor Discovery message",
            NdErr::HopLimit => "hop limit is not 255: it did not come from this link",
            NdErr::Source => "the source is not a link-local address",
            NdErr::DadDest => "a probe for address detection must go to a solicited-node group",
            NdErr::SourceLla => "an unspecified source with a link-layer address option",
            NdErr::SolicitedToGroup => "a solicited advertisement to a multicast group",
        }
    }
}

/// a Neighbor Discovery message with its options
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Nd {
    pub msg: NdMsg,
    pub opts: Vec<NdOpt>,
}

/// the options of `b` (the bytes after the fixed part): each with its offset and length in
/// `b`; Err when one has length 0 or runs past the end
pub fn options(b: &[u8]) -> Result<Vec<(NdOpt, usize, usize)>, NdErr> {
    let mut out = Vec::new();
    let mut p = 0;
    while p < b.len() {
        if p + 2 > b.len() {
            return Err(NdErr::BadOption);
        }
        let (t, l) = (b[p], b[p + 1] as usize * 8);
        if l == 0 || p + l > b.len() {
            return Err(NdErr::BadOption);
        }
        let v = &b[p + 2..p + l];
        let o = match t {
            OPT_SRC_LLA if v.len() >= 6 => NdOpt::SrcLla(Mac(v[..6].try_into().unwrap_or([0; 6]))),
            OPT_TGT_LLA if v.len() >= 6 => NdOpt::TgtLla(Mac(v[..6].try_into().unwrap_or([0; 6]))),
            OPT_PREFIX if l == 32 => NdOpt::Prefix(PrefixInfo {
                len: v[0],
                on_link: v[1] & 0x80 != 0,
                auto: v[1] & 0x40 != 0,
                valid: be32(v, 2),
                preferred: be32(v, 6),
                prefix: Ipv6(v[14..30].try_into().unwrap_or([0; 16])),
            }),
            OPT_REDIRECTED if v.len() >= 6 => NdOpt::Redirected(v[6..].to_vec()),
            OPT_MTU if l == 8 => NdOpt::Mtu(be32(v, 2)),
            _ => NdOpt::Other(t, v.to_vec()),
        };
        out.push((o, p, l));
        p += l;
    }
    Ok(out)
}

/// the length of the fixed part of an ND message type
pub fn fixed_len(typ: u8) -> Option<usize> {
    Some(match typ {
        T_ROUTER_SOLICIT => RS_LEN,
        T_ROUTER_ADVERT => RA_LEN,
        T_NEIGHBOR_SOLICIT => NS_LEN,
        T_NEIGHBOR_ADVERT => NA_LEN,
        T_REDIRECT => REDIRECT_LEN,
        _ => return None,
    })
}

fn addr_at(b: &[u8], off: usize) -> Ipv6 {
    Ipv6(b[off..off + 16].try_into().unwrap_or([0; 16]))
}

impl Nd {
    pub fn new(msg: NdMsg, opts: Vec<NdOpt>) -> Nd {
        Nd { msg, opts }
    }
    /// a Router Solicitation (with the sender's MAC unless it has no address yet)
    pub fn rs(src_lla: Option<Mac>) -> Nd {
        Nd { msg: NdMsg::RouterSolicit, opts: src_lla.map(NdOpt::SrcLla).into_iter().collect() }
    }
    /// a Neighbor Solicitation (DAD probes carry no source link-layer address)
    pub fn ns(target: Ipv6, src_lla: Option<Mac>) -> Nd {
        Nd { msg: NdMsg::NeighborSolicit { target }, opts: src_lla.map(NdOpt::SrcLla).into_iter().collect() }
    }
    /// a Neighbor Advertisement
    pub fn na(target: Ipv6, router: bool, solicited: bool, override_: bool, tgt_lla: Option<Mac>) -> Nd {
        Nd { msg: NdMsg::NeighborAdvert { router, solicited, override_, target }, opts: tgt_lla.map(NdOpt::TgtLla).into_iter().collect() }
    }

    /// reads an ND message (the checks RFC 4861 makes on the message itself)
    pub fn parse(v: &Icmp6View) -> Result<Nd, NdErr> {
        let b = v.bytes();
        let fixed = fixed_len(v.typ()).ok_or(NdErr::NotNd)?;
        if v.code() != 0 {
            return Err(NdErr::Code(v.code()));
        }
        if b.len() < fixed {
            return Err(NdErr::Short);
        }
        let msg = match v.typ() {
            T_ROUTER_SOLICIT => NdMsg::RouterSolicit,
            T_ROUTER_ADVERT => NdMsg::RouterAdvert(Ra {
                hop_limit: b[4],
                managed: b[5] & 0x80 != 0,
                other: b[5] & 0x40 != 0,
                prf: match (b[5] >> 3) & 3 {
                    2 => 0,
                    p => p,
                },
                lifetime: be16(b, 6),
                reachable: be32(b, 8),
                retrans: be32(b, 12),
            }),
            T_NEIGHBOR_SOLICIT => NdMsg::NeighborSolicit { target: addr_at(b, 8) },
            T_NEIGHBOR_ADVERT => NdMsg::NeighborAdvert { router: b[4] & 0x80 != 0, solicited: b[4] & 0x40 != 0, override_: b[4] & 0x20 != 0, target: addr_at(b, 8) },
            _ => NdMsg::Redirect { target: addr_at(b, 8), dst: addr_at(b, 24) },
        };
        match msg {
            NdMsg::NeighborSolicit { target } | NdMsg::NeighborAdvert { target, .. } if target.is_multicast() => return Err(NdErr::Multicast),
            NdMsg::Redirect { dst, .. } if dst.is_multicast() => return Err(NdErr::Multicast),
            _ => {}
        }
        let opts = options(&b[fixed..])?.into_iter().map(|(o, _, _)| o).collect();
        Ok(Nd { msg, opts })
    }

    pub fn typ(&self) -> u8 {
        match self.msg {
            NdMsg::RouterSolicit => T_ROUTER_SOLICIT,
            NdMsg::RouterAdvert(_) => T_ROUTER_ADVERT,
            NdMsg::NeighborSolicit { .. } => T_NEIGHBOR_SOLICIT,
            NdMsg::NeighborAdvert { .. } => T_NEIGHBOR_ADVERT,
            NdMsg::Redirect { .. } => T_REDIRECT,
        }
    }

    /// the message with its checksum
    pub fn encode(&self, src: Ipv6, dst: Ipv6) -> Vec<u8> {
        let mut v: Vec<u8> = alloc::vec![self.typ(), 0, 0, 0];
        match self.msg {
            NdMsg::RouterSolicit => v.extend_from_slice(&[0; 4]),
            NdMsg::RouterAdvert(r) => {
                let prf = (r.prf & 3) << 3;
                v.push(r.hop_limit);
                v.push(if r.managed { 0x80 } else { 0 } | if r.other { 0x40 } else { 0 } | prf);
                v.extend_from_slice(&r.lifetime.to_be_bytes());
                v.extend_from_slice(&r.reachable.to_be_bytes());
                v.extend_from_slice(&r.retrans.to_be_bytes());
            }
            NdMsg::NeighborSolicit { target } => {
                v.extend_from_slice(&[0; 4]);
                v.extend_from_slice(&target.0);
            }
            NdMsg::NeighborAdvert { router, solicited, override_, target } => {
                v.push(if router { 0x80 } else { 0 } | if solicited { 0x40 } else { 0 } | if override_ { 0x20 } else { 0 });
                v.extend_from_slice(&[0; 3]);
                v.extend_from_slice(&target.0);
            }
            NdMsg::Redirect { target, dst } => {
                v.extend_from_slice(&[0; 4]);
                v.extend_from_slice(&target.0);
                v.extend_from_slice(&dst.0);
            }
        }
        for o in &self.opts {
            o.write(&mut v);
        }
        seal(&mut v, src, dst);
        v
    }

    /// the source link-layer address option
    pub fn src_lla(&self) -> Option<Mac> {
        self.opts.iter().find_map(|o| if let NdOpt::SrcLla(m) = o { Some(*m) } else { None })
    }
    /// the target link-layer address option
    pub fn tgt_lla(&self) -> Option<Mac> {
        self.opts.iter().find_map(|o| if let NdOpt::TgtLla(m) = o { Some(*m) } else { None })
    }
    /// the prefix information options
    pub fn prefixes(&self) -> impl Iterator<Item = &PrefixInfo> {
        self.opts.iter().filter_map(|o| if let NdOpt::Prefix(p) = o { Some(p) } else { None })
    }
    /// the MTU option
    pub fn mtu(&self) -> Option<u32> {
        self.opts.iter().find_map(|o| if let NdOpt::Mtu(m) = o { Some(*m) } else { None })
    }

    /// the rules of RFC 4861 that need the packet's addresses and hop limit: hop limit 255 for
    /// every message, a link-local source for RA and redirect, an unspecified source only with a
    /// solicited-node destination and without a source link-layer address (NS) or without it
    /// (RS), solicited NA never to a group; None when the message may be taken
    pub fn check_ip(&self, src: Ipv6, dst: Ipv6, hop_limit: u8) -> Option<NdErr> {
        if hop_limit != 255 {
            return Some(NdErr::HopLimit);
        }
        match self.msg {
            NdMsg::RouterAdvert(_) | NdMsg::Redirect { .. } if !src.is_link_local() => Some(NdErr::Source),
            NdMsg::NeighborSolicit { .. } if src.is_unspecified() && !dst.is_solicited_node() => Some(NdErr::DadDest),
            NdMsg::NeighborSolicit { .. } | NdMsg::RouterSolicit if src.is_unspecified() && self.src_lla().is_some() => Some(NdErr::SourceLla),
            NdMsg::NeighborAdvert { solicited: true, .. } if dst.is_multicast() => Some(NdErr::SolicitedToGroup),
            _ => None,
        }
    }
}

/// a name of an ICMPv6 type and code as capture tools write it
pub fn name(typ: u8, code: u8) -> &'static str {
    match (typ, code) {
        (T_DEST_UNREACH, UNREACH_NO_ROUTE) => "Destination Unreachable (no route to destination)",
        (T_DEST_UNREACH, UNREACH_ADMIN) => "Destination Unreachable (communication with destination administratively prohibited)",
        (T_DEST_UNREACH, UNREACH_BEYOND_SCOPE) => "Destination Unreachable (beyond scope of source address)",
        (T_DEST_UNREACH, UNREACH_ADDR) => "Destination Unreachable (address unreachable)",
        (T_DEST_UNREACH, UNREACH_PORT) => "Destination Unreachable (port unreachable)",
        (T_DEST_UNREACH, UNREACH_POLICY) => "Destination Unreachable (source address failed ingress/egress policy)",
        (T_DEST_UNREACH, UNREACH_REJECT) => "Destination Unreachable (reject route to destination)",
        (T_DEST_UNREACH, _) => "Destination Unreachable",
        (T_PACKET_TOO_BIG, _) => "Packet Too Big",
        (T_TIME_EXCEEDED, EXCEEDED_HOP_LIMIT) => "Time Exceeded (hop limit exceeded in transit)",
        (T_TIME_EXCEEDED, EXCEEDED_REASSEMBLY) => "Time Exceeded (fragment reassembly time exceeded)",
        (T_TIME_EXCEEDED, _) => "Time Exceeded",
        (T_PARAM_PROBLEM, PARAM_HEADER) => "Parameter Problem (erroneous header field encountered)",
        (T_PARAM_PROBLEM, PARAM_NEXT_HEADER) => "Parameter Problem (unrecognized Next Header type encountered)",
        (T_PARAM_PROBLEM, PARAM_OPTION) => "Parameter Problem (unrecognized IPv6 option encountered)",
        (T_PARAM_PROBLEM, _) => "Parameter Problem",
        (T_ECHO_REQUEST, _) => "Echo (ping) request",
        (T_ECHO_REPLY, _) => "Echo (ping) reply",
        (T_MLD_QUERY, _) => "Multicast Listener Query",
        (T_MLD_REPORT, _) => "Multicast Listener Report",
        (T_MLD_DONE, _) => "Multicast Listener Done",
        (T_ROUTER_SOLICIT, _) => "Router Solicitation",
        (T_ROUTER_ADVERT, _) => "Router Advertisement",
        (T_NEIGHBOR_SOLICIT, _) => "Neighbor Solicitation",
        (T_NEIGHBOR_ADVERT, _) => "Neighbor Advertisement",
        (T_REDIRECT, _) => "Redirect",
        (T_MLD2_REPORT, _) => "Multicast Listener Report Message v2",
        _ => "ICMPv6",
    }
}
