// l3 -- what devices do with IPv4: ARP, local delivery, forwarding, fragmentation and ICMP
//
//   iface_up(n, i)            an interface carries traffic: power, admin state, link (ports),
//                             a live port in the VLAN (SVIs)
//   l3_from_port / l3_from_svi    a frame for the device's own IP part (from a routed port or
//                             from its bridge); tagged frames go to the port's 802.1Q
//                             subinterface of their VLAN; serial ports carry Cisco HDLC
//   ip_output(n, packet, origin)  route lookup, MTU (fragment or "fragmentation needed"),
//                             next hop resolution with ARP, framing
//   ip_input / ip_deliver     on IOS-like devices with access lists or NAT, the inbound list
//                             of the interface and the translation from outside come between
//                             the checks of a packet and its way to the device or onward;
//                             ip_output translates and applies the outbound list after the
//                             route lookup, ip_send does the rest (acl.rs, nat.rs: the order)
//   icmp_error                RFC 1812 4.3.2.7 rules; limited as Linux (50 at once, 1000/s,
//                             6 at once per destination, then 1/s) or IOS (an unreachable
//                             every 500 ms) does
//   l3_timer                  ARP retries (3 requests a second apart, then "host
//                             unreachable" for what waited, also for the device's own TCP
//                             connections), reassembly expiry (30 s)
//
// While ARP asks, Linux keeps up to 101 packets per neighbour; IOS drops the packet that
// needs the answer (the first "." of ".!!!!"), so it never reports "host unreachable".  IOS
// interfaces answer ARP for hosts the router reaches over another interface (proxy ARP).
//
// Hosts follow the weak host model (any own address on any interface); broadcast pings are
// ignored as Linux does; UDP goes to the socket of its port (os.rs) and gets "port
// unreachable" when nobody listens; a DHCP client's packet socket also takes datagrams to an
// address the device does not have yet; TCP goes to tcp.rs (a RST where nobody listens, ICMP
// errors about own segments to their connection).  Time Exceeded and the
// unreachables come from the address of the interface the packet arrived on (as IOS does;
// traceroute shows the inbound interfaces); Linux quotes up to 576 bytes of the packet, IOS
// its header and 8 bytes.  Loopback addresses never come from a wire (martians); on Linux
// devices all of 127.0.0.0/8 is the device itself.

use alloc::sync::Arc;
use alloc::vec::Vec;

use jfn_model::catalog::Media;
use netpkt::arp::{Arp, ArpView, OP_REQUEST};
use netpkt::eth::{self, EthHdr, VlanTag, ET_ARP, ET_IPV4, L2};
use netpkt::icmp::{self, Icmp, IcmpView};
use netpkt::ipv4::{self, Ipv4Hdr, Ipv4View, PROTO_ICMP, PROTO_TCP, PROTO_UDP};
use netpkt::pkt::{Pkt, HEADROOM};
use netpkt::udp::UdpView;
use netpkt::{Ipv4, Mac};

use crate::ip::{ArpEntry, IfKind, IpStack, NextHop, Origin, Pending, ARP_TRIES, REASM_TIME, SLOT_ARP, SLOT_REASM};
use crate::jobs::IcmpKind;
use crate::kernel::{Engine, Frame, SEC};
use crate::trace::{HopId, Note};

/// the first bytes of a Cisco HDLC frame carrying IPv4 (address, control, protocol)
pub const HDLC_IPV4: [u8; 4] = [0x0F, 0x00, 0x08, 0x00];

/// a forwarded packet as it arrived (forwarding took one off its TTL already): what an ICMP
/// error about it quotes
pub(crate) fn as_received(pkt: &[u8]) -> Vec<u8> {
    let mut q = pkt.to_vec();
    if q.len() >= 20 {
        let hl = ((q[0] & 0x0F) as usize * 4).clamp(20, q.len());
        q[8] = q[8].wrapping_add(1);
        q[10] = 0;
        q[11] = 0;
        let c = netpkt::checksum::checksum(&q[..hl]);
        q[10..12].copy_from_slice(&c.to_be_bytes());
    }
    q
}

/// what to do with a packet for a neighbour
enum L2Step {
    Send(Mac),
    Queued,
    Request,
}

impl Engine {
    pub(crate) fn stack(&self, ni: usize) -> Option<&IpStack> {
        self.nodes.get(ni)?.as_ref()?.ip.as_ref()
    }
    pub(crate) fn stack_mut(&mut self, ni: usize) -> Option<&mut IpStack> {
        self.nodes.get_mut(ni)?.as_mut()?.ip.as_mut()
    }
    pub(crate) fn dev_of(&self, ni: usize) -> u32 {
        match self.nodes.get(ni) {
            Some(Some(n)) => n.dev,
            _ => 0,
        }
    }

    /// the interface carries traffic
    pub(crate) fn iface_up(&self, ni: usize, i: u16) -> bool {
        let Some(Some(n)) = self.nodes.get(ni) else {
            return false;
        };
        if !n.power {
            return false;
        }
        let Some(f) = n.ip.as_ref().and_then(|s| s.ifaces.get(i as usize)) else {
            return false;
        };
        if f.hidden {
            return false;
        }
        match f.kind {
            IfKind::Port(p) => {
                let member = n.bridge.as_ref().is_some_and(|b| b.ports.get(p as usize).is_some_and(|bp| bp.member));
                !member && n.ports.get(p as usize).is_some_and(|port| port.admin_up && port.link.is_some_and(|li| self.link_up(li)))
            }
            IfKind::Sub { port: p, .. } => {
                let member = n.bridge.as_ref().is_some_and(|b| b.ports.get(p as usize).is_some_and(|bp| bp.member));
                f.admin_up && !member && n.ports.get(p as usize).is_some_and(|port| port.admin_up && port.link.is_some_and(|li| self.link_up(li)))
            }
            IfKind::Svi(v) => {
                f.admin_up
                    && n.bridge.as_ref().is_some_and(|b| (0..b.ports.len() as u16).any(|p| b.carries(p, v) && n.ports[p as usize].link.is_some_and(|li| self.link_up(li))))
            }
            IfKind::Loopback(_) => f.admin_up,
        }
    }

    /// the next hop to `dst` from node `ni`
    pub(crate) fn route(&self, ni: usize, dst: Ipv4) -> Option<NextHop> {
        let st = self.stack(ni)?;
        st.lookup(&|i| self.iface_up(ni, i), dst)
    }

    /// the address a packet to `dst` leaves with (the primary address of the outgoing
    /// interface)
    pub(crate) fn source_for(&self, ni: usize, dst: Ipv4) -> Option<Ipv4> {
        let st = self.stack(ni)?;
        if st.is_mine(dst) {
            return Some(dst);
        }
        let nh = self.route(ni, dst)?;
        st.ifaces.get(nh.iface as usize)?.primary()
    }

    /// `dst` is for this device (an own address, the limited broadcast, or the directed
    /// broadcast of the receiving interface)
    fn is_local(&self, ni: usize, iface: u16, dst: Ipv4) -> bool {
        let Some(st) = self.stack(ni) else {
            return false;
        };
        dst.is_broadcast() || st.is_mine(dst) || st.ifaces.get(iface as usize).is_some_and(|f| f.is_broadcast(dst))
    }

    // ---------------------------------------------------------------- input

    /// a frame on a routed port
    pub(crate) fn l3_from_port(&mut self, ni: usize, pi: u16, frame: Frame, hop: HopId) {
        let Some(Some(n)) = self.nodes.get(ni) else {
            return;
        };
        let dev = n.dev;
        let media = n.ports[pi as usize].spec.media;
        let Some((iface, mac)) = n.ip.as_ref().and_then(|s| s.iface_of_port(pi).map(|i| (i, s.ifaces[i as usize].mac))) else {
            self.note(dev, Some(hop), Note::NotForMe { port: pi });
            return;
        };
        if media == Media::Serial {
            if frame.len() > 4 && frame[..4] == HDLC_IPV4 {
                self.ip_input(ni, iface, &frame[4..], hop);
            } else {
                self.note(dev, Some(hop), Note::DropMalformed { port: pi });
            }
            return;
        }
        let Some(l2) = L2::parse(&frame) else {
            self.note(dev, Some(hop), Note::DropMalformed { port: pi });
            return;
        };
        // a tag picks the subinterface (priority tags, VLAN 0, belong to the port itself)
        let iface = match l2.tag {
            Some(t) if t.vid != 0 => match self.stack(ni).and_then(|s| s.iface_of_sub(pi, t.vid)) {
                Some(sub) => sub,
                None => {
                    self.note(dev, Some(hop), Note::DropVlan { port: pi, vid: t.vid });
                    return;
                }
            },
            _ => iface,
        };
        if l2.dst != mac && !l2.dst.is_multicast() {
            self.note(dev, Some(hop), Note::NotForMe { port: pi });
            return;
        }
        let payload = l2.payload(&frame);
        match l2.ethertype {
            ET_ARP => match ArpView::parse(payload) {
                Some(a) => self.arp_input(ni, iface, Arp::read(&a), hop),
                None => self.note(dev, Some(hop), Note::DropMalformed { port: pi }),
            },
            ET_IPV4 => self.ip_input(ni, iface, payload, hop),
            _ => self.note(dev, Some(hop), Note::Delivered { port: pi }),
        }
    }

    /// a frame the bridge hands to its own interface in a VLAN (untagged)
    pub(crate) fn l3_from_svi(&mut self, ni: usize, iface: u16, frame: &[u8], hop: HopId) {
        let Some(l2) = L2::parse(frame) else {
            return;
        };
        match l2.ethertype {
            ET_ARP => {
                if let Some(a) = ArpView::parse(l2.payload(frame)) {
                    self.arp_input(ni, iface, Arp::read(&a), hop);
                }
            }
            ET_IPV4 => self.ip_input(ni, iface, l2.payload(frame), hop),
            _ => {}
        }
    }

    fn arp_input(&mut self, ni: usize, iface: u16, a: Arp, hop: HopId) {
        // address conflict detection of services (DHCP clients probing their address)
        self.os_arp_seen(ni, iface, &a);
        let now = self.now;
        let dev = self.dev_of(ni);
        let Some((st, f)) = self.stack(ni).and_then(|st| Some((st, st.ifaces.get(iface as usize)?))) else {
            return;
        };
        // proxy ARP (RFC 1027): a router answers for a host it reaches over another interface
        // and then acts as the target would
        let proxy = !f.has(a.tpa)
            && a.op == OP_REQUEST
            && f.proxy_arp
            && st.forwarding
            && f.on_link(a.spa)
            && !f.on_link(a.tpa)
            && !a.tpa.is_unspecified()
            && !a.tpa.is_multicast()
            && !a.tpa.is_broadcast()
            && self.route(ni, a.tpa).is_some_and(|nh| nh.iface != iface);
        // the global addresses of NAT in the network of an outside interface (nat.rs)
        let nat = a.op == OP_REQUEST && self.nat_arp(ni, iface, a.tpa);
        let Some(st) = self.stack_mut(ni) else {
            return;
        };
        let f = &st.ifaces[iface as usize];
        let mine = f.has(a.tpa) || nat;
        let my_mac = f.mac;
        let svi = matches!(f.kind, IfKind::Svi(_));
        let mut flushed: Option<(bool, Vec<Pending>)> = None;
        if !a.spa.is_unspecified() {
            let key = (iface, a.spa);
            // RFC 826: the target merges or adds the sender; others only refresh what they have
            if mine || proxy || st.arp.contains_key(&key) {
                let e = st.arp.entry(key).or_insert(ArpEntry { mac: None, at: now, tries: 0, queue: Vec::new(), fixed: false });
                if !e.fixed {
                    let changed = e.mac != Some(a.sha);
                    e.mac = Some(a.sha);
                    e.at = now;
                    e.tries = 0;
                    flushed = Some((changed, core::mem::take(&mut e.queue)));
                }
            }
        }
        if let Some((changed, queue)) = flushed {
            if changed {
                self.note(dev, Some(hop), Note::ArpLearned { ip: a.spa, mac: a.sha });
            }
            for p in queue {
                self.frame_out(ni, iface, a.sha, ET_IPV4, &p.pkt, p.cause);
            }
        }
        if mine && a.op == OP_REQUEST {
            let reply = Arp::reply_to(&a, my_mac, a.tpa);
            let mut b = [0u8; 28];
            reply.write(&mut b);
            self.note(dev, Some(hop), Note::ArpReply { iface, ip: a.tpa, mac: my_mac });
            self.frame_out(ni, iface, a.sha, ET_ARP, &b, Some(hop));
        } else if proxy {
            let reply = Arp::reply_to(&a, my_mac, a.tpa);
            let mut b = [0u8; 28];
            reply.write(&mut b);
            self.note(dev, Some(hop), Note::ArpProxied { iface, ip: a.tpa });
            self.frame_out(ni, iface, a.sha, ET_ARP, &b, Some(hop));
        } else if a.op == OP_REQUEST && !svi {
            // every host of the segment hears the question and the others drop it (a bridge's
            // own interface only gets a copy of what it floods anyway)
            self.note(dev, Some(hop), Note::ArpIgnored { ip: a.tpa });
        }
    }

    fn ip_input(&mut self, ni: usize, iface: u16, bytes: &[u8], hop: HopId) {
        let dev = self.dev_of(ni);
        let Some(v) = Ipv4View::parse(bytes) else {
            self.note(dev, Some(hop), Note::IpMalformed);
            return;
        };
        if !v.checksum_ok() {
            self.note(dev, Some(hop), Note::IpBadChecksum);
            return;
        }
        let (src, dst) = (v.src(), v.dst());
        // RFC 1122 3.2.1.3, RFC 1812 5.3.7: loopback addresses never come from a wire, and
        // nobody sends from a group or broadcast address
        if src.is_loopback() || dst.is_loopback() || src.is_multicast() || src.is_broadcast() {
            self.note(dev, Some(hop), Note::IpMartian { src, dst });
            return;
        }
        // an injected partition: packets from the other side are dropped (fault.rs)
        if self.faults.any_partition() && self.partition_drop(ni, src) {
            self.note(dev, Some(hop), Note::PartitionDrop { src });
            return;
        }
        // the inbound access list of the interface (acl.rs), then on an outside interface the
        // translation of the destination back (nat.rs)
        if self.stack(ni).is_some_and(|s| !s.acls.bound.is_empty() || s.nat.active()) {
            if !self.acl_pass(ni, iface, false, v.packet(), v.packet(), iface, Some(hop)) {
                return;
            }
            if self.nat_in(ni, iface, &v, hop) {
                return;
            }
        }
        self.ip_deliver(ni, iface, &v, hop);
    }

    /// a packet that passed the checks of its interface: for the device itself, or forwarded
    pub(crate) fn ip_deliver(&mut self, ni: usize, iface: u16, v: &Ipv4View, hop: HopId) {
        let dev = self.dev_of(ni);
        let dst = v.dst();
        if self.is_local(ni, iface, dst) {
            if v.is_fragment() {
                self.reassemble(ni, iface, v, hop);
            } else {
                self.ip_local(ni, iface, v.packet().to_vec(), hop);
            }
            return;
        }
        // a DHCP client reads its port before it has an address (a packet socket)
        if self.os_raw_udp(ni, iface, v, hop) {
            return;
        }
        if dst.is_multicast() || dst.is_unspecified() {
            return;
        }
        if !self.stack(ni).is_some_and(|s| s.forwarding) {
            self.note(dev, Some(hop), Note::IpNotForMe { dst });
            return;
        }
        if v.ttl() <= 1 {
            self.note(dev, Some(hop), Note::IpTtlExpired { src: v.src() });
            self.icmp_error(ni, iface, v.packet(), Icmp::TimeExceeded { code: icmp::EXCEEDED_TTL }, Some(hop));
            return;
        }
        let mut p = v.packet().to_vec();
        let hl = v.header_len();
        ipv4::dec_ttl(&mut p[..hl]);
        self.ip_output(ni, p, Origin::Forwarded { iface }, Some(hop));
    }

    fn reassemble(&mut self, ni: usize, iface: u16, v: &Ipv4View, hop: HopId) {
        let now = self.now;
        let dev = self.dev_of(ni);
        let key = ipv4::reassembly_key(v);
        let Some(st) = self.stack_mut(ni) else {
            return;
        };
        let e = st.reasm.entry(key).or_insert_with(|| (ipv4::Reassembly::new(), now + REASM_TIME));
        let full = e.0.add(v);
        if full.is_some() {
            st.reasm.remove(&key);
        }
        let next = st.reasm.values().map(|e| e.1).min();
        match full {
            Some(pkt) => {
                self.note(dev, Some(hop), Note::IpReassembled { bytes: pkt.len() as u32 });
                self.ip_local(ni, iface, pkt, hop);
            }
            None => {
                if let Some(t) = next {
                    self.set_timer(ni, SLOT_REASM, t);
                }
            }
        }
    }

    /// a packet (reassembled) for this device
    pub(crate) fn ip_local(&mut self, ni: usize, iface: u16, pkt: Vec<u8>, hop: HopId) {
        let dev = self.dev_of(ni);
        let Some(v) = Ipv4View::parse(&pkt) else {
            return;
        };
        self.note(dev, Some(hop), Note::IpLocal { proto: v.proto() });
        let bcast = v.dst().is_broadcast() || self.stack(ni).and_then(|s| s.ifaces.get(iface as usize)).is_some_and(|f| f.is_broadcast(v.dst()));
        match v.proto() {
            PROTO_ICMP => self.icmp_input(ni, iface, &v, bcast, hop),
            PROTO_UDP => {
                let Some(u) = UdpView::parse(v.payload()) else {
                    return;
                };
                if !u.checksum_ok_v4(v.src(), v.dst()) {
                    self.note(dev, Some(hop), Note::IpBadChecksum);
                    return;
                }
                if self.udp_input(ni, iface, &v, &u, hop) {
                    return;
                }
                self.note(dev, Some(hop), Note::UdpNoPort { port: u.dst_port() });
                if !bcast {
                    self.icmp_error(ni, iface, v.packet(), Icmp::Unreachable { code: icmp::UNREACH_PORT, mtu: 0 }, Some(hop));
                }
            }
            PROTO_TCP => self.tcp_input(ni, &v, bcast, hop),
            _ => {
                if !bcast {
                    self.icmp_error(ni, iface, v.packet(), Icmp::Unreachable { code: icmp::UNREACH_PROTO, mtu: 0 }, Some(hop));
                }
            }
        }
    }

    fn icmp_input(&mut self, ni: usize, _iface: u16, v: &Ipv4View, bcast: bool, hop: HopId) {
        let dev = self.dev_of(ni);
        let Some(ic) = IcmpView::parse(v.payload()) else {
            return;
        };
        if !ic.checksum_ok() {
            self.note(dev, Some(hop), Note::IpBadChecksum);
            return;
        }
        match Icmp::read(&ic) {
            Icmp::EchoRequest { id, seq } => {
                if bcast {
                    return;
                }
                let Some(st) = self.stack_mut(ni) else {
                    return;
                };
                let (ttl, ipid) = (st.ttl, st.next_id());
                let mut p = Pkt::new(HEADROOM, ic.payload());
                Icmp::EchoReply { id, seq }.push_onto(&mut p);
                let mut h = Ipv4Hdr::new(v.dst(), v.src(), PROTO_ICMP, ttl);
                h.id = ipid;
                h.push_onto(&mut p);
                self.note(dev, Some(hop), Note::EchoAnswered { to: v.src() });
                self.ip_output(ni, p.into_vec(), Origin::Local { job: None }, Some(hop));
            }
            Icmp::EchoReply { id, seq } => {
                self.jobs_echo_reply(ni, v.src(), id, seq, v.ttl(), v.total_len() as u16);
                self.os_echo_reply(ni, v.src(), id, seq);
            }
            _ => {
                if ic.is_error()
                    && let Some(q) = ic.quoted()
                {
                    self.jobs_icmp_error(ni, v.src(), ic.typ(), ic.code(), u16::from_be_bytes([ic.rest()[2], ic.rest()[3]]), &q);
                    self.tcp_icmp_error(ni, ic.typ(), ic.code(), u16::from_be_bytes([ic.rest()[2], ic.rest()[3]]), &q);
                }
            }
        }
    }

    // ---------------------------------------------------------------- output

    /// sends an IPv4 packet made or forwarded by node `ni`
    pub(crate) fn ip_output(&mut self, ni: usize, pkt: Vec<u8>, origin: Origin, cause: Option<HopId>) {
        let dev = self.dev_of(ni);
        let Some(v) = Ipv4View::parse(&pkt) else {
            return;
        };
        let (dst, src) = (v.dst(), v.src());
        // to itself (Linux: anything in 127.0.0.0/8 is the device itself)
        if let Origin::Local { .. } = origin
            && let Some(st) = self.stack(ni)
            && let Some(i) = st.ifaces.iter().position(|f| !f.hidden && (f.has(dst) || (f.is_loopback() && dst.is_loopback() && f.on_link(dst))))
        {
            let h = cause.unwrap_or(0);
            self.ip_local(ni, i as u16, pkt, h);
            return;
        }
        let nh = if dst.is_broadcast() {
            self.stack(ni).and_then(|st| {
                let i = st
                    .ifaces
                    .iter()
                    .position(|f| !f.is_loopback() && f.has(src))
                    .or_else(|| (0..st.ifaces.len()).find(|&i| !st.ifaces[i].is_loopback() && self.iface_up(ni, i as u16)))?;
                Some(NextHop { iface: i as u16, gw: dst, src: crate::ip::RouteSrc::Connected })
            })
        } else {
            self.route(ni, dst)
        };
        let Some(nh) = nh else {
            self.note(dev, cause, Note::IpNoRoute { dst });
            match origin {
                Origin::Local { job: Some((j, seq)) } => self.job_error(j, seq, Ipv4::UNSPECIFIED, IcmpKind::NoRoute),
                Origin::Local { job: None } => {}
                Origin::Forwarded { iface } => self.icmp_error(ni, iface, &as_received(&pkt), Icmp::Unreachable { code: icmp::UNREACH_NET, mtu: 0 }, cause),
            }
            return;
        };
        if let Origin::Forwarded { .. } = origin {
            // "no ip directed-broadcast" (IOS since 12.0), bc_forwarding 0 (Linux)
            if self.stack(ni).and_then(|s| s.ifaces.get(nh.iface as usize)).is_some_and(|f| f.is_broadcast(dst)) {
                self.note(dev, cause, Note::IpDirectedBroadcast { dst });
                return;
            }
            self.note(dev, cause, Note::IpRouted { dst, gw: nh.gw, iface: nh.iface });
        }
        if !self.stack(ni).is_some_and(|s| !s.acls.bound.is_empty() || s.nat.active()) {
            self.ip_send(ni, nh, pkt, None, origin, cause);
            return;
        }
        // NAT on the way out (nat.rs), then the outbound access list of the interface for what
        // the device forwards (acl.rs; it sees the translated packet, its ICMP answer quotes the
        // packet as it arrived)
        for (p, orig, c) in self.nat_out(ni, origin, nh.iface, pkt, cause) {
            if let Origin::Forwarded { iface } = origin {
                let q = as_received(orig.as_deref().unwrap_or(&p));
                if !self.acl_pass(ni, nh.iface, true, &p, &q, iface, c) {
                    continue;
                }
            }
            self.ip_send(ni, nh, p, orig, origin, c);
        }
    }

    /// the MTU of the way out (fragments, or "fragmentation needed"), then the neighbour;
    /// `orig`: the packet before NAT changed it, which an ICMP error quotes
    fn ip_send(&mut self, ni: usize, nh: NextHop, pkt: Vec<u8>, orig: Option<Vec<u8>>, origin: Origin, cause: Option<HopId>) {
        let dev = self.dev_of(ni);
        let Some(v) = Ipv4View::parse(&pkt) else {
            return;
        };
        let (dst, df, len) = (v.dst(), v.df(), v.total_len());
        let mtu = self.stack(ni).and_then(|s| s.ifaces.get(nh.iface as usize)).map(|f| f.mtu as usize).unwrap_or(1500);
        let parts: Vec<Vec<u8>> = if len > mtu {
            if df {
                self.note(dev, cause, Note::IpFragNeeded { mtu: mtu as u16 });
                match origin {
                    Origin::Local { job: Some((j, seq)) } => {
                        let from = self.source_for(ni, dst).unwrap_or(Ipv4::UNSPECIFIED);
                        self.job_error(j, seq, from, IcmpKind::FragNeeded(mtu as u16));
                    }
                    Origin::Local { job: None } => {}
                    Origin::Forwarded { iface } => self.icmp_error(ni, iface, &as_received(orig.as_deref().unwrap_or(&pkt)), Icmp::Unreachable { code: icmp::UNREACH_FRAG_NEEDED, mtu: mtu as u16 }, cause),
                }
                return;
            }
            match ipv4::fragment(&pkt, mtu) {
                Some(f) => {
                    self.note(dev, cause, Note::IpFragmented { parts: f.len() as u16, mtu: mtu as u16 });
                    f
                }
                None => return,
            }
        } else {
            alloc::vec![pkt]
        };
        for p in parts {
            self.l2_send(ni, nh.iface, nh.gw, p, origin, cause);
        }
    }

    /// hands a packet to the neighbour `gw` behind interface `iface`
    fn l2_send(&mut self, ni: usize, iface: u16, gw: Ipv4, pkt: Vec<u8>, origin: Origin, cause: Option<HopId>) {
        let now = self.now;
        let dev = self.dev_of(ni);
        let Some(Some(n)) = self.nodes.get(ni) else {
            return;
        };
        let Some(st) = n.ip.as_ref() else {
            return;
        };
        let Some(f) = st.ifaces.get(iface as usize) else {
            return;
        };
        let serial = matches!(f.kind, IfKind::Port(p) if n.ports[p as usize].spec.media == Media::Serial);
        if serial {
            self.frame_out(ni, iface, Mac::BROADCAST, ET_IPV4, &pkt, cause);
            return;
        }
        if matches!(f.kind, IfKind::Loopback(_)) {
            return;
        }
        if gw.is_broadcast() || f.is_broadcast(gw) {
            self.frame_out(ni, iface, Mac::BROADCAST, ET_IPV4, &pkt, cause);
            return;
        }
        if gw.is_multicast() {
            self.frame_out(ni, iface, Mac::of_ipv4_multicast(gw), ET_IPV4, &pkt, cause);
            return;
        }
        let (timeout, qmax) = (st.arp_timeout, st.arp_queue);
        let key = (iface, gw);
        let step = match st.arp.get(&key) {
            Some(e) if e.mac.is_some() && (e.fixed || now.saturating_sub(e.at) <= timeout) => L2Step::Send(e.mac.unwrap_or(Mac::BROADCAST)),
            Some(e) if e.mac.is_none() => L2Step::Queued,
            _ => L2Step::Request,
        };
        match step {
            L2Step::Send(mac) => self.frame_out(ni, iface, mac, ET_IPV4, &pkt, cause),
            L2Step::Queued if qmax == 0 => self.note(dev, cause, Note::ArpIncomplete { ip: gw }),
            L2Step::Queued => {
                if let Some(e) = self.stack_mut(ni).and_then(|s| s.arp.get_mut(&key)) {
                    if e.queue.len() >= qmax {
                        e.queue.remove(0);
                    }
                    e.queue.push(Pending { pkt, origin, cause });
                }
                self.note(dev, cause, Note::ArpWaiting { ip: gw });
            }
            L2Step::Request => {
                let queue = if qmax > 0 { alloc::vec![Pending { pkt, origin, cause }] } else { Vec::new() };
                if let Some(st) = self.stack_mut(ni) {
                    st.arp.insert(key, ArpEntry { mac: None, at: now, tries: 1, queue, fixed: false });
                }
                self.arp_request(ni, iface, gw, cause);
                if qmax == 0 {
                    self.note(dev, cause, Note::ArpIncomplete { ip: gw });
                }
                self.arm_arp(ni);
            }
        }
    }

    fn arp_request(&mut self, ni: usize, iface: u16, target: Ipv4, cause: Option<HopId>) {
        let dev = self.dev_of(ni);
        let Some(f) = self.stack(ni).and_then(|s| s.ifaces.get(iface as usize)) else {
            return;
        };
        let (mac, src) = (f.mac, f.primary().unwrap_or(Ipv4::UNSPECIFIED));
        let mut b = [0u8; 28];
        Arp::request(mac, src, target).write(&mut b);
        self.note(dev, cause, Note::ArpRequest { iface, ip: target });
        self.frame_out(ni, iface, Mac::BROADCAST, ET_ARP, &b, cause);
    }

    /// the ARP timer at the earliest retry of any open request
    fn arm_arp(&mut self, ni: usize) {
        let next = self.stack(ni).and_then(|s| s.arp.values().filter(|e| e.mac.is_none()).map(|e| e.at + SEC).min());
        if let Some(t) = next {
            self.set_timer(ni, SLOT_ARP, t);
        }
    }

    /// frames a payload for interface `iface` (Ethernet, padded; Cisco HDLC on serial ports;
    /// through the bridge for SVIs)
    pub(crate) fn frame_out(&mut self, ni: usize, iface: u16, dst: Mac, ethertype: u16, payload: &[u8], cause: Option<HopId>) {
        let Some(Some(n)) = self.nodes.get(ni) else {
            return;
        };
        let Some(f) = n.ip.as_ref().and_then(|s| s.ifaces.get(iface as usize)) else {
            return;
        };
        let (kind, mac) = (f.kind, f.mac);
        match kind {
            IfKind::Port(p) => {
                if n.ports[p as usize].spec.media == Media::Serial {
                    let mut fr = Vec::with_capacity(payload.len() + 4);
                    fr.extend_from_slice(&[0x0F, 0x00]);
                    fr.extend_from_slice(&ethertype.to_be_bytes());
                    fr.extend_from_slice(payload);
                    self.send(ni, p, Arc::from(fr), cause);
                } else {
                    let mut fr = EthHdr { dst, src: mac, ethertype }.frame(payload);
                    eth::pad(&mut fr);
                    self.send(ni, p, Arc::from(fr), cause);
                }
            }
            IfKind::Sub { port: p, vlan } => {
                let mut fr = EthHdr { dst, src: mac, ethertype }.frame(payload);
                eth::pad(&mut fr);
                self.send(ni, p, Arc::from(eth::tagged(&fr, VlanTag::new(vlan))), cause);
            }
            IfKind::Svi(vlan) => {
                let mut fr = EthHdr { dst, src: mac, ethertype }.frame(payload);
                eth::pad(&mut fr);
                self.bridge_output(ni, vlan, Arc::from(fr), cause);
            }
            IfKind::Loopback(_) => {}
        }
    }

    /// an ICMP error about `orig` (which arrived on `in_iface`) back to its sender
    pub(crate) fn icmp_error(&mut self, ni: usize, in_iface: u16, orig: &[u8], msg: Icmp, cause: Option<HopId>) {
        let now = self.now;
        let dev = self.dev_of(ni);
        let Some(ov) = Ipv4View::parse_truncated(orig) else {
            return;
        };
        let s = ov.src();
        // RFC 1812 4.3.2.7: never about errors, broadcasts, multicasts, later fragments or
        // impossible sources
        if ov.frag_off() > 0 || s.is_unspecified() || s.is_broadcast() || s.is_multicast() || s.is_loopback() || ov.dst().is_broadcast() || ov.dst().is_multicast() {
            return;
        }
        if ov.proto() == PROTO_ICMP && IcmpView::parse(ov.payload()).is_some_and(|q| q.is_error()) {
            return;
        }
        let Some(st) = self.stack_mut(ni) else {
            return;
        };
        let (typ, code) = msg.type_code();
        if !st.icmp_allow(now, typ, code, s) {
            self.note(dev, cause, Note::IcmpLimited);
            return;
        }
        let from = st
            .ifaces
            .get(in_iface as usize)
            .and_then(|f| f.primary())
            .or_else(|| st.ifaces.iter().filter(|f| !f.hidden).flat_map(|f| f.addrs.iter()).map(|p| p.addr).find(|a| !a.is_loopback()));
        let Some(from) = from else {
            return;
        };
        let (ttl, ipid) = (st.ttl, st.next_id());
        let quote = if st.long_quote { &orig[..ov.total_len().min(orig.len()).min(576 - 28)] } else { icmp::error_quote(orig) };
        let mut p = Pkt::new(HEADROOM, quote);
        msg.push_onto(&mut p);
        let mut h = Ipv4Hdr::new(from, s, PROTO_ICMP, ttl);
        h.id = ipid;
        h.push_onto(&mut p);
        self.note(dev, cause, Note::IcmpSent { typ, code, to: s });
        self.ip_output(ni, p.into_vec(), Origin::Local { job: None }, cause);
    }

    // ---------------------------------------------------------------- timers

    pub(crate) fn l3_timer(&mut self, ni: usize, slot: u32) {
        let now = self.now;
        let dev = self.dev_of(ni);
        match slot {
            SLOT_ARP => {
                let Some(st) = self.stack_mut(ni) else {
                    return;
                };
                let mut retry = Vec::new();
                let mut failed = Vec::new();
                for (k, e) in st.arp.iter_mut() {
                    if e.mac.is_none() && now >= e.at + SEC {
                        if e.tries >= ARP_TRIES {
                            failed.push((*k, core::mem::take(&mut e.queue)));
                        } else {
                            e.tries += 1;
                            e.at = now;
                            retry.push(*k);
                        }
                    }
                }
                for (k, _) in &failed {
                    st.arp.remove(k);
                }
                for (iface, ip) in retry {
                    self.arp_request(ni, iface, ip, None);
                }
                for ((iface, ip), queue) in failed {
                    self.note(dev, None, Note::ArpFailed { ip });
                    for p in queue {
                        self.arp_failed(ni, iface, p);
                    }
                }
                self.arm_arp(ni);
            }
            SLOT_REASM => {
                let Some(st) = self.stack_mut(ni) else {
                    return;
                };
                let n = st.reasm.len();
                st.reasm.retain(|_, e| e.1 > now);
                let gone = n - st.reasm.len();
                let next = st.reasm.values().map(|e| e.1).min();
                for _ in 0..gone {
                    self.note(dev, None, Note::IpReasmTimeout);
                }
                if let Some(t) = next {
                    self.set_timer(ni, SLOT_REASM, t);
                }
            }
            _ => {}
        }
    }

    /// a port lost its link: what ARP learned behind it is forgotten (IOS clears the
    /// interface's entries, Linux flushes on carrier loss), and so is the learning of SVIs
    /// that have no live port left
    pub(crate) fn l3_port_down(&mut self, ni: usize, pi: u16) {
        let Some(st) = self.stack(ni) else {
            return;
        };
        let gone: Vec<u16> = (0..st.ifaces.len() as u16)
            .filter(|&i| match st.ifaces[i as usize].kind {
                IfKind::Port(p) | IfKind::Sub { port: p, .. } => p == pi,
                IfKind::Svi(_) => !self.iface_up(ni, i),
                IfKind::Loopback(_) => false,
            })
            .collect();
        if let Some(st) = self.stack_mut(ni) {
            for i in gone {
                st.flush_arp(i);
            }
        }
    }

    /// a packet whose neighbour never answered ARP
    fn arp_failed(&mut self, ni: usize, iface: u16, p: Pending) {
        match p.origin {
            Origin::Local { job: Some((j, seq)) } => {
                let from = self.stack(ni).and_then(|s| s.ifaces.get(iface as usize)).and_then(|f| f.primary()).unwrap_or(Ipv4::UNSPECIFIED);
                self.job_error(j, seq, from, IcmpKind::HostUnreachable);
            }
            Origin::Local { job: None } => {
                // the device's own TCP hears of it as of an ICMP error: a connection attempt
                // ends with "No route to host" (Linux)
                if let Some(v) = Ipv4View::parse(&p.pkt)
                    && v.proto() == PROTO_TCP
                {
                    self.tcp_icmp_error(ni, icmp::T_UNREACH, icmp::UNREACH_HOST, 0, &v);
                }
            }
            Origin::Forwarded { iface: inif } => self.icmp_error(ni, inif, &as_received(&p.pkt), Icmp::Unreachable { code: icmp::UNREACH_HOST, mtu: 0 }, p.cause),
        }
    }
}
