// l3v6 -- what devices do with IPv6 packets (RFC 8200, 4443): input from a port, a
// subinterface, a serial line or the bridge; local delivery; forwarding with the hop limit;
// output with source address selection and fragmentation by the sender; ICMPv6 errors with the
// limits of Linux or IOS
//
//   ip6_input(ni, iface, bytes, hop)      checks (version, IPv6 running on the interface, no
//                                         group or loopback source), then local delivery, a
//                                         group nobody here listens to, or forwarding
//   ip6_output(ni, pkt, oif, origin, cause)   a packet the device made: to itself, to a group
//                                         (on `oif`), else the route (`oif` for link-local
//                                         destinations) and the path MTU; a packet larger than
//                                         that is fragmented with a fragment header unless the
//                                         ping forbids it
//   icmp6_error(ni, in_iface, orig, msg, cause)   never about errors or redirects, never to
//                                         unspecified, group or loopback sources, about packets
//                                         to a group only Packet Too Big and Parameter Problem
//                                         code 2 (RFC 4443 2.4 (e)); the source is a global
//                                         address of the receiving interface, else the one the
//                                         way back uses, else its link-local address; the quote
//                                         fills the minimum MTU
//   ip6_source(ni, dst, oif)              RFC 6724 in short: the address itself, the scope of
//                                         the destination, not deprecated, the outgoing
//                                         interface, the longest common prefix
//   ip6_route(ni, dst, oif) / ip6_mtu(ni, iface)
//
// Routers never fragment (Packet Too Big with the MTU of the next link, RFC 8201) and never
// forward link-local addresses: a destination stays on its link, a link-local source going to
// another link gets "beyond scope of source address".  Packets to a tentative address are
// dropped (RFC 4862 5.4).  Hosts follow the weak host model for global addresses.  There are
// no IPv6 sockets yet: UDP gets "port unreachable" (so traceroute ends at its target), TCP a
// RST; another next header a Parameter Problem pointing at it.

use alloc::vec::Vec;

use jfn_model::catalog::Media;
use netpkt::eth::ET_IPV6;
use netpkt::icmpv6::{self, Icmp6, Icmp6View};
use netpkt::ipv6::{self, Ipv6Hdr, Ipv6View, ReasmErr, IPV6_HDR, MIN_MTU, NH_ICMPV6, NH_NONE, NH_TCP, NH_UDP};
use netpkt::pkt::{Pkt, HEADROOM};
use netpkt::tcp::{self, TcpHdr, TcpOpts, TcpView};
use netpkt::udp::UdpView;
use netpkt::{Ipv6, Mac};

use crate::ip::{IfKind, Origin};
use crate::ip6::{Ip6, Limit6, NextHop6, IOS_ERR_BUCKET, IOS_ERR_INTERVAL, REASM_TIME6, SLOT_V6_REASM};
use crate::kernel::{Engine, Ns, MS, SEC};
use crate::trace::{HopId, Note};
use crate::trace6::Note6;

/// Linux: errors of the whole device (icmp_msgs_burst, icmp_msgs_per_sec)
const LINUX_BURST: u32 = 50;
const LINUX_PER_SEC: u64 = 1000;

impl Engine {
    pub(crate) fn v6(&self, ni: usize) -> Option<&Ip6> {
        self.stack(ni).map(|s| &s.v6)
    }
    pub(crate) fn v6_mut(&mut self, ni: usize) -> Option<&mut Ip6> {
        self.stack_mut(ni).map(|s| &mut s.v6)
    }
    /// a note about IPv6
    pub(crate) fn note6(&mut self, ni: usize, hop: Option<HopId>, n: Note6) {
        let dev = self.dev_of(ni);
        self.note(dev, hop, Note::V6(n));
    }
    /// IPv6 runs on interface `i` and the interface carries traffic
    pub(crate) fn ip6_running(&self, ni: usize, i: u16) -> bool {
        self.v6(ni).is_some_and(|v| v.running(i)) && self.iface_up(ni, i)
    }
    /// the next hop to `dst` (`oif`: the interface of a link-local destination)
    pub(crate) fn ip6_route(&self, ni: usize, dst: Ipv6, oif: Option<u16>) -> Option<NextHop6> {
        let v = self.v6(ni)?;
        v.lookup(&|i| self.iface_up(ni, i), self.now, dst, oif)
    }
    /// the IPv6 MTU of an interface: "ipv6 mtu", else what advertisements said, else the link's
    pub(crate) fn ip6_mtu(&self, ni: usize, i: u16) -> usize {
        let link = self.stack(ni).and_then(|s| s.ifaces.get(i as usize)).map(|f| f.mtu as usize).unwrap_or(1500);
        let f = self.v6(ni).and_then(|v| v.iface(i));
        let m = f.and_then(|f| f.mtu).map(|m| m as usize).or_else(|| f.and_then(|f| f.link_mtu).map(|m| (m as usize).min(link))).unwrap_or(link);
        m.max(MIN_MTU)
    }
    /// the hop limit of packets the device makes on an interface (what advertisements said, or
    /// the device's)
    pub(crate) fn ip6_hops(&self, ni: usize, i: Option<u16>) -> u8 {
        let v = self.v6(ni);
        i.and_then(|i| v.and_then(|v| v.iface(i)).and_then(|f| f.hop_limit)).or(v.map(|v| v.hop_limit)).unwrap_or(64)
    }

    /// the source address for a packet to `dst`
    pub(crate) fn ip6_source(&self, ni: usize, dst: Ipv6, oif: Option<u16>) -> Option<Ipv6> {
        let v = self.v6(ni)?;
        if v.find(dst).is_some() {
            return Some(dst);
        }
        let out = oif.or_else(|| self.ip6_route(ni, dst, None).map(|nh| nh.iface));
        if dst.is_link_local() || (dst.is_multicast() && dst.mc_scope() <= 2) {
            return out.and_then(|i| v.iface(i)).and_then(|f| f.link_local());
        }
        let mut best: Option<((bool, bool, bool, u8), Ipv6)> = None;
        for (i, f) in v.ifs.iter().enumerate() {
            if !f.up || !self.iface_up(ni, i as u16) {
                continue;
            }
            for a in &f.addrs {
                if !a.state.usable() || a.addr.is_link_local() || a.anycast || a.addr.is_loopback() {
                    continue;
                }
                let score = (true, a.state == crate::ip6::AddrState::Preferred, out == Some(i as u16), a.addr.common_prefix(&dst));
                if best.as_ref().is_none_or(|b| score > b.0) {
                    best = Some((score, a.addr));
                }
            }
        }
        best.map(|b| b.1).or_else(|| out.and_then(|i| v.iface(i)).and_then(|f| f.link_local()))
    }

    /// `dst` is for this device on interface `iface`: a group it joined, its link-local address
    /// there, a usable address of any interface (weak host)
    fn ip6_mine(&self, ni: usize, iface: u16, dst: Ipv6) -> bool {
        let Some(v) = self.v6(ni) else {
            return false;
        };
        if dst.is_multicast() {
            return v.joined(iface, dst);
        }
        if dst.is_link_local() {
            return v.iface(iface).is_some_and(|f| f.owns(dst));
        }
        v.find(dst).is_some()
    }

    // ---------------------------------------------------------------- input

    /// an IPv6 packet that arrived on interface `iface`
    pub(crate) fn ip6_input(&mut self, ni: usize, iface: u16, bytes: &[u8], hop: HopId) {
        let Some(v) = Ipv6View::parse(bytes) else {
            self.note6(ni, Some(hop), Note6::Malformed);
            return;
        };
        if !self.ip6_running(ni, iface) {
            self.note6(ni, Some(hop), Note6::Off { iface });
            return;
        }
        let (src, dst) = (v.src(), v.dst());
        // RFC 4291 2.5.3, 2.7: nobody sends from a group or the loopback address, and ::1 never
        // comes from a wire
        if src.is_multicast() || src.is_loopback() || dst.is_loopback() || dst.is_unspecified() {
            self.note6(ni, Some(hop), Note6::Martian { src });
            return;
        }
        if self.ip6_mine(ni, iface, dst) {
            self.ip6_deliver(ni, iface, &v, hop);
            return;
        }
        let Some(v6) = self.v6(ni) else {
            return;
        };
        if dst.is_multicast() {
            self.note6(ni, Some(hop), Note6::GroupIgnored { dst });
            return;
        }
        // an address that is still tentative (or a duplicate) takes nothing (RFC 4862 5.4)
        let tentative = v6.ifs.iter().any(|f| f.has(dst));
        if tentative || !v6.routing {
            self.note6(ni, Some(hop), Note6::NotForMe { dst });
            return;
        }
        self.ip6_forward(ni, iface, &v, hop);
    }

    /// a packet for this device: whole, or a fragment to put together
    fn ip6_deliver(&mut self, ni: usize, iface: u16, v: &Ipv6View, hop: HopId) {
        match ipv6::frag(v) {
            Some((f, at)) if f.offset > 0 || f.more => self.reassemble6(ni, iface, v, f, at, hop),
            _ => self.ip6_local(ni, iface, v.packet().to_vec(), hop),
        }
    }

    fn reassemble6(&mut self, ni: usize, iface: u16, v: &Ipv6View, f: ipv6::FragHdr, at: usize, hop: HopId) {
        let now = self.now;
        let key = ipv6::reassembly_key(v, &f);
        let Some(v6) = self.v6_mut(ni) else {
            return;
        };
        let e = v6.reasm.entry(key).or_insert_with(|| (ipv6::Reassembly6::new(), now + REASM_TIME6));
        let r = e.0.add(v, &f, at);
        if matches!(r, Ok(Some(_))) {
            v6.reasm.remove(&key);
        }
        let next = v6.reasm_next();
        match r {
            Ok(Some(pkt)) => {
                self.note6(ni, Some(hop), Note6::Reassembled { bytes: pkt.len() as u32 });
                self.ip6_local(ni, iface, pkt, hop);
            }
            Ok(None) => {}
            Err(e) => {
                self.note6(ni, Some(hop), Note6::ReasmBad);
                // RFC 8200 4.5: a bad length or offset gets a Parameter Problem, an overlap
                // nothing (RFC 5722)
                let pointer = match e {
                    ReasmErr::BadLength => Some(4),
                    ReasmErr::TooLong => Some((IPV6_HDR + at + 2) as u32),
                    ReasmErr::Overlap => None,
                };
                if let Some(p) = pointer {
                    let pkt = v.packet().to_vec();
                    self.icmp6_error(ni, iface, &pkt, Icmp6::ParamProblem { code: icmpv6::PARAM_HEADER, pointer: p }, Some(hop));
                }
            }
        }
        self.arm6(ni, SLOT_V6_REASM, next);
    }

    /// a whole packet for this device
    pub(crate) fn ip6_local(&mut self, ni: usize, iface: u16, pkt: Vec<u8>, hop: HopId) {
        let Some(v) = Ipv6View::parse(&pkt) else {
            return;
        };
        let Some((nh, off)) = ipv6::upper_layer(&v) else {
            self.note6(ni, Some(hop), Note6::Malformed);
            return;
        };
        self.note6(ni, Some(hop), Note6::Local { nh });
        let body = &v.payload()[off..];
        let group = v.dst().is_multicast();
        match nh {
            NH_ICMPV6 => self.icmp6_input(ni, iface, &v, body, hop),
            NH_UDP => {
                let Some(u) = UdpView::parse(body) else {
                    return;
                };
                if !u.checksum_ok_v6(v.src(), v.dst()) {
                    self.note6(ni, Some(hop), Note6::BadChecksum);
                    return;
                }
                self.note6(ni, Some(hop), Note6::UdpNoPort { port: u.dst_port() });
                if !group {
                    self.icmp6_error(ni, iface, v.packet(), Icmp6::DestUnreach { code: icmpv6::UNREACH_PORT }, Some(hop));
                }
            }
            NH_TCP => self.tcp6_reset(ni, &v, body, hop),
            NH_NONE => {}
            _ => {
                if !group {
                    // the Next Header field that names it: the fixed header's, or that of the
                    // last extension header
                    let at = ipv6::ext_headers(&v).and_then(|e| e.last().map(|x| IPV6_HDR + x.off)).unwrap_or(6);
                    self.icmp6_error(ni, iface, v.packet(), Icmp6::ParamProblem { code: icmpv6::PARAM_NEXT_HEADER, pointer: at as u32 }, Some(hop));
                }
            }
        }
    }

    /// TCP over IPv6 has no sockets yet: every segment but a RST gets one (RFC 9293 3.10.7.1)
    fn tcp6_reset(&mut self, ni: usize, v: &Ipv6View, body: &[u8], hop: HopId) {
        let Some(t) = TcpView::parse(body) else {
            return;
        };
        if !t.checksum_ok_v6(v.src(), v.dst()) {
            self.note6(ni, Some(hop), Note6::BadChecksum);
            return;
        }
        if t.has(tcp::RST) || v.dst().is_multicast() {
            return;
        }
        self.note6(ni, Some(hop), Note6::TcpReset { port: t.dst_port() });
        let len = t.payload().len() as u32 + t.has(tcp::SYN) as u32 + t.has(tcp::FIN) as u32;
        let (seq, ack, flags) = if t.has(tcp::ACK) { (t.ack(), 0, tcp::RST) } else { (0, t.seq().wrapping_add(len), tcp::RST | tcp::ACK) };
        let h = TcpHdr { src_port: t.dst_port(), dst_port: t.src_port(), seq, ack, flags, window: 0, urgent: 0, opts: TcpOpts::default() };
        let mut p = Pkt::new(HEADROOM, &[]);
        h.push_v6(&mut p, v.dst(), v.src());
        let hops = self.ip6_hops(ni, None);
        Ipv6Hdr::new(v.dst(), v.src(), NH_TCP, hops).push_onto(&mut p);
        let oif = if v.src().is_link_local() { self.v6(ni).and_then(|x| x.find(v.dst())) } else { None };
        self.ip6_output(ni, p.into_vec(), oif, Origin::Local { job: None }, Some(hop));
    }

    fn icmp6_input(&mut self, ni: usize, iface: u16, v: &Ipv6View, body: &[u8], hop: HopId) {
        let Some(ic) = Icmp6View::parse(body) else {
            return;
        };
        let (src, dst) = (v.src(), v.dst());
        if !ic.checksum_ok(src, dst) {
            self.note6(ni, Some(hop), Note6::BadChecksum);
            return;
        }
        match ic.typ() {
            icmpv6::T_ECHO_REQUEST => {
                // groups are answered too (Linux echo_ignore_multicast 0): from an address of
                // the interface
                let from = if dst.is_multicast() { self.ip6_source(ni, src, Some(iface)) } else { Some(dst) };
                let Some(from) = from else {
                    return;
                };
                let oif = if src.is_link_local() { Some(iface) } else { None };
                let hops = self.ip6_hops(ni, Some(iface));
                let mut p = Pkt::new(HEADROOM, ic.body());
                Icmp6::EchoReply { id: ic.id(), seq: ic.seq() }.push_v6(&mut p, from, src);
                Ipv6Hdr::new(from, src, NH_ICMPV6, hops).push_onto(&mut p);
                self.note6(ni, Some(hop), Note6::EchoAnswered { to: src });
                self.ip6_output(ni, p.into_vec(), oif, Origin::Local { job: None }, Some(hop));
            }
            icmpv6::T_ECHO_REPLY => self.jobs6_echo_reply(ni, src, ic.id(), ic.seq(), v.hop_limit(), ic.len() as u16),
            t if t < 128 => {
                if let Some(q) = ic.quoted() {
                    if t == icmpv6::T_PACKET_TOO_BIG {
                        self.pmtu_learn(ni, q.dst(), ic.value(), hop);
                    }
                    self.jobs6_icmp_error(ni, src, t, ic.code(), ic.value(), &q);
                }
            }
            t if (icmpv6::T_ROUTER_SOLICIT..=icmpv6::T_REDIRECT).contains(&t) => self.nd_input(ni, iface, v, &ic, hop),
            _ => {}
        }
    }

    /// Packet Too Big about one of the device's packets: the path MTU to its destination
    /// (RFC 8201: never below 1280, forgotten after 10 minutes)
    fn pmtu_learn(&mut self, ni: usize, dst: Ipv6, mtu: u32, hop: HopId) {
        let now = self.now;
        let mtu = (mtu as usize).clamp(MIN_MTU, 65535) as u16;
        let Some(v6) = self.v6_mut(ni) else {
            return;
        };
        let old = v6.pmtu.get(&dst).map(|x| x.0);
        if old.is_some_and(|o| o <= mtu) {
            return;
        }
        v6.pmtu.insert(dst, (mtu, now + crate::ip6::PMTU_AGE));
        let next = v6.reasm_next();
        self.note6(ni, Some(hop), Note6::PmtuLearned { dst, mtu });
        self.arm6(ni, SLOT_V6_REASM, next);
    }

    // ---------------------------------------------------------------- forwarding

    fn ip6_forward(&mut self, ni: usize, iface: u16, v: &Ipv6View, hop: HopId) {
        let (src, dst) = (v.src(), v.dst());
        if dst.is_link_local() {
            self.note6(ni, Some(hop), Note6::LinkScope { src, dst });
            return;
        }
        if v.hop_limit() <= 1 {
            self.note6(ni, Some(hop), Note6::HopLimit { src });
            self.icmp6_error(ni, iface, v.packet(), Icmp6::TimeExceeded { code: icmpv6::EXCEEDED_HOP_LIMIT }, Some(hop));
            return;
        }
        let Some(nh) = self.ip6_route(ni, dst, None) else {
            self.note6(ni, Some(hop), Note6::NoRoute { dst });
            self.icmp6_error(ni, iface, v.packet(), Icmp6::DestUnreach { code: icmpv6::UNREACH_NO_ROUTE }, Some(hop));
            return;
        };
        if src.is_link_local() && nh.iface != iface {
            self.note6(ni, Some(hop), Note6::LinkScope { src, dst });
            self.icmp6_error(ni, iface, v.packet(), Icmp6::DestUnreach { code: icmpv6::UNREACH_BEYOND_SCOPE }, Some(hop));
            return;
        }
        let mtu = self.ip6_mtu(ni, nh.iface);
        if v.packet().len() > mtu {
            self.note6(ni, Some(hop), Note6::TooBig { mtu: mtu as u16 });
            self.icmp6_error(ni, iface, v.packet(), Icmp6::PacketTooBig { mtu: mtu as u32 }, Some(hop));
            return;
        }
        let mut p = v.packet().to_vec();
        ipv6::dec_hop_limit(&mut p);
        self.note6(ni, Some(hop), Note6::Routed { dst, gw: nh.gw, iface: nh.iface });
        self.ip6_l2(ni, nh, p, Origin::Forwarded { iface }, Some(hop));
    }

    // ---------------------------------------------------------------- output

    /// sends a packet made by node `ni` (`oif`: the interface for a link-local or link-scope
    /// group destination, or one the sender insists on)
    pub(crate) fn ip6_output(&mut self, ni: usize, pkt: Vec<u8>, oif: Option<u16>, origin: Origin, cause: Option<HopId>) {
        let Some(v) = Ipv6View::parse(&pkt) else {
            return;
        };
        let dst = v.dst();
        // to itself
        if !dst.is_multicast()
            && let Some(i) = self.v6(ni).and_then(|x| x.find(dst))
        {
            self.ip6_local(ni, i, pkt, cause.unwrap_or(0));
            return;
        }
        if dst.is_multicast() {
            let i = oif.or_else(|| self.v6(ni).and_then(|x| x.find(v.src())));
            if let Some(i) = i.filter(|&i| self.ip6_running(ni, i)) {
                self.ip6_l2(ni, NextHop6 { iface: i, gw: dst, src: crate::ip6::Src6::Connected }, pkt, origin, cause);
            }
            return;
        }
        let Some(nh) = self.ip6_route(ni, dst, oif) else {
            self.note6(ni, cause, Note6::NoRoute { dst });
            if let Origin::Local { job: Some((j, seq)) } = origin {
                self.job6_error(j, seq, Ipv6::UNSPECIFIED, crate::jobs6::Icmp6Kind::LocalNoRoute);
            }
            return;
        };
        // the sender fragments what does not fit the path (RFC 8200 5)
        let path = self.v6(ni).and_then(|x| x.pmtu.get(&dst)).map(|p| p.0 as usize);
        let mtu = self.ip6_mtu(ni, nh.iface).min(path.unwrap_or(usize::MAX));
        if pkt.len() > mtu {
            if let Origin::Local { job: Some((j, seq)) } = origin
                && self.job6_no_frag(j)
            {
                self.job6_error(j, seq, Ipv6::UNSPECIFIED, crate::jobs6::Icmp6Kind::TooBig(mtu as u32));
                return;
            }
            let id = match self.v6_mut(ni) {
                Some(x) => {
                    x.frag_id = x.frag_id.wrapping_add(1);
                    x.frag_id
                }
                None => return,
            };
            let parts = ipv6::fragment(&pkt, mtu, id).unwrap_or_default();
            if parts.is_empty() {
                return;
            }
            self.note6(ni, cause, Note6::Fragmented { parts: parts.len() as u16, mtu: mtu as u16 });
            for p in parts {
                self.ip6_l2(ni, nh, p, origin, cause);
            }
            return;
        }
        self.ip6_l2(ni, nh, pkt, origin, cause);
    }

    /// hands a packet to the neighbor `nh.gw` behind `nh.iface`: a group MAC, a serial line, or
    /// the neighbor cache
    pub(crate) fn ip6_l2(&mut self, ni: usize, nh: NextHop6, pkt: Vec<u8>, origin: Origin, cause: Option<HopId>) {
        let Some(Some(n)) = self.nodes.get(ni) else {
            return;
        };
        let Some(f) = n.ip.as_ref().and_then(|s| s.ifaces.get(nh.iface as usize)) else {
            return;
        };
        let serial = matches!(f.kind, IfKind::Port(p) if n.ports[p as usize].spec.media == Media::Serial);
        if matches!(f.kind, IfKind::Loopback(_)) {
            return;
        }
        if serial {
            self.frame_out(ni, nh.iface, Mac::BROADCAST, ET_IPV6, &pkt, cause);
            return;
        }
        if nh.gw.is_multicast() {
            self.frame_out(ni, nh.iface, Mac::of_ipv6_multicast(nh.gw), ET_IPV6, &pkt, cause);
            return;
        }
        self.nd_send(ni, nh.iface, nh.gw, pkt, origin, cause);
    }

    // ---------------------------------------------------------------- ICMPv6 errors

    /// may an error of this type go to `to` now?  (Takes what it costs.)
    fn icmp6_allow(&mut self, ni: usize, typ: u8, to: Ipv6) -> bool {
        let now = self.now;
        // the prefix length of the route back (Linux gives wider prefixes more)
        let plen = self.v6(ni).map(|v| v.rib(&|i| self.iface_up(ni, i), now)).and_then(|rib| rib.iter().filter(|r| r.prefix.contains(to)).map(|r| r.prefix.len).max()).unwrap_or(0);
        let Some(v6) = self.v6_mut(ni) else {
            return false;
        };
        let r = &mut v6.rate;
        match v6.limit {
            Limit6::Ios => {
                if !r.started {
                    r.started = true;
                    r.tokens = IOS_ERR_BUCKET;
                    r.at = now;
                }
                let add = now.saturating_sub(r.at) / IOS_ERR_INTERVAL;
                if add > 0 {
                    r.tokens = (r.tokens as u64 + add).min(IOS_ERR_BUCKET as u64) as u32;
                    r.at += add * IOS_ERR_INTERVAL;
                }
                if r.tokens == 0 {
                    return false;
                }
                r.tokens -= 1;
                true
            }
            Limit6::Linux => {
                // Packet Too Big is not limited at all (net.ipv6.icmp.ratemask 0-1,3-127)
                if typ == icmpv6::T_PACKET_TOO_BIG {
                    return true;
                }
                if !r.started {
                    r.started = true;
                    r.tokens = LINUX_BURST;
                    r.at = now;
                }
                let add = now.saturating_sub(r.at) * LINUX_PER_SEC / SEC;
                if add > 0 {
                    r.tokens = (r.tokens as u64 + add).min(LINUX_BURST as u64) as u32;
                    r.at += add * SEC / LINUX_PER_SEC;
                }
                if r.tokens == 0 {
                    return false;
                }
                let shift = if plen < 128 { (128 - plen as u32) >> 5 } else { 0 };
                let tmo: Ns = (1000u64 >> shift) * MS;
                let max = 6 * tmo;
                let (credit, at) = r.peers.get(&to).copied().unwrap_or((max, now));
                let credit = (credit + now.saturating_sub(at)).min(max);
                if credit < tmo {
                    r.peers.insert(to, (credit, now));
                    return false;
                }
                r.peers.insert(to, (credit - tmo, now));
                r.tokens -= 1;
                true
            }
        }
    }

    /// an ICMPv6 error about `orig` (which arrived on `in_iface`) back to its sender
    pub(crate) fn icmp6_error(&mut self, ni: usize, in_iface: u16, orig: &[u8], msg: Icmp6, cause: Option<HopId>) {
        let Some(ov) = Ipv6View::parse_truncated(orig) else {
            return;
        };
        let (s, d) = (ov.src(), ov.dst());
        if s.is_unspecified() || s.is_multicast() || s.is_loopback() {
            return;
        }
        let exempt = matches!(msg, Icmp6::PacketTooBig { .. } | Icmp6::ParamProblem { code: icmpv6::PARAM_OPTION, .. });
        if d.is_multicast() && !exempt {
            return;
        }
        // never about an error or a redirect
        if let Some((nh, off)) = ipv6::upper_layer(&ov)
            && nh == NH_ICMPV6
            && ov.payload().get(off).is_some_and(|&t| t < 128 || t == icmpv6::T_REDIRECT)
        {
            return;
        }
        let (typ, code) = msg.type_code();
        if !self.icmp6_allow(ni, typ, s) {
            self.note6(ni, cause, Note6::IcmpLimited);
            return;
        }
        let from = self
            .v6(ni)
            .and_then(|v| v.iface(in_iface))
            .and_then(|f| f.addrs.iter().find(|a| a.state.usable() && a.addr.is_global_unicast() && !a.anycast).map(|a| a.addr))
            .or_else(|| self.ip6_source(ni, s, if s.is_link_local() { Some(in_iface) } else { None }))
            .or_else(|| self.v6(ni).and_then(|v| v.iface(in_iface)).and_then(|f| f.link_local()));
        let Some(from) = from else {
            return;
        };
        let hops = self.ip6_hops(ni, None);
        let mut p = Pkt::new(HEADROOM, icmpv6::error_quote(orig));
        msg.push_v6(&mut p, from, s);
        Ipv6Hdr::new(from, s, NH_ICMPV6, hops).push_onto(&mut p);
        self.note6(ni, cause, Note6::IcmpSent { typ, code, to: s });
        let oif = if s.is_link_local() { Some(in_iface) } else { None };
        self.ip6_output(ni, p.into_vec(), oif, Origin::Local { job: None }, cause);
    }

    /// fragments given up after 60 s (Time Exceeded code 1 when the first one had come, RFC
    /// 8200 4.5), path MTUs tried again after 10 minutes
    pub(crate) fn reasm6_timer(&mut self, ni: usize) {
        let now = self.now;
        let Some(v6) = self.v6_mut(ni) else {
            return;
        };
        let gone: Vec<(ipv6::Reassembly6, (Ipv6, Ipv6, u32))> = v6.reasm.iter().filter(|(_, e)| e.1 <= now).map(|(k, e)| (e.0.clone(), *k)).collect();
        v6.reasm.retain(|_, e| e.1 > now);
        v6.pmtu.retain(|_, p| p.1 > now);
        let next = v6.reasm_next();
        for (r, (src, _, _)) in gone {
            self.note6(ni, None, Note6::ReasmTimeout);
            if let Some(first) = r.first_packet()
                && let Some(i) = self.ip6_route(ni, src, None).map(|nh| nh.iface)
            {
                self.icmp6_error(ni, i, &first, Icmp6::TimeExceeded { code: icmpv6::EXCEEDED_REASSEMBLY }, None);
            }
        }
        self.arm6(ni, SLOT_V6_REASM, next);
    }
}
