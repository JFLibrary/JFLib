// jobs6 -- ping and traceroute over IPv6: ICMPv6 echo, UDP probes with growing hop limits
//
//   Engine::ping6(dev, dst, Ping6Opts) -> JobId     echo requests, one per interval (0: the
//                                                   next when the previous is settled, as IOS);
//                                                   a group destination counts the first
//                                                   answer of each probe
//   Engine::traceroute6(dev, dst, TraceOpts, oif) -> JobId   UDP to port 33434 + n with hop
//                                                   limit 1, 2, ...: Time Exceeded names the
//                                                   hops, Port Unreachable the target
//   Ping6Ev / Trace6Probe / Icmp6Kind              what the consoles print (cli/print6.rs)
//
// The jobs live with the IPv4 ones (JobKind::Ping6, JobKind::Trace6: the same ids, events,
// Ctrl+C and console binding).  Round trip times are virtual: from handing the probe to IPv6
// (neighbor discovery included) until the answer is delivered.

use alloc::collections::BTreeMap;
use alloc::vec::Vec;

use netpkt::icmpv6::{self, Icmp6};
use netpkt::ipv6::{Ipv6Hdr, Ipv6View, NH_ICMPV6, NH_UDP};
use netpkt::pkt::{Pkt, HEADROOM};
use netpkt::udp::UdpHdr;
use netpkt::{be16, Ipv6};

use crate::action::ActErr;
use crate::ip::Origin;
use crate::jobs::{Job, JobId, JobKind, TraceOpts};
use crate::kernel::{Engine, Ev, Ns, SEC};

/// why a probe failed
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icmp6Kind {
    /// Destination Unreachable with its code (RFC 4443 3.1)
    NoRoute,
    Prohibited,
    BeyondScope,
    AddrUnreachable,
    PortUnreachable,
    Policy,
    Reject,
    /// Packet Too Big with the MTU (also the sender's own "message too long")
    TooBig(u32),
    HopLimit,
    Reassembly,
    /// Parameter Problem: code and pointer
    Param(u8, u32),
    /// the sending device itself has no route
    LocalNoRoute,
    Other(u8, u8),
}

impl Icmp6Kind {
    pub fn of(typ: u8, code: u8, value: u32) -> Icmp6Kind {
        match (typ, code) {
            (icmpv6::T_DEST_UNREACH, icmpv6::UNREACH_NO_ROUTE) => Icmp6Kind::NoRoute,
            (icmpv6::T_DEST_UNREACH, icmpv6::UNREACH_ADMIN) => Icmp6Kind::Prohibited,
            (icmpv6::T_DEST_UNREACH, icmpv6::UNREACH_BEYOND_SCOPE) => Icmp6Kind::BeyondScope,
            (icmpv6::T_DEST_UNREACH, icmpv6::UNREACH_ADDR) => Icmp6Kind::AddrUnreachable,
            (icmpv6::T_DEST_UNREACH, icmpv6::UNREACH_PORT) => Icmp6Kind::PortUnreachable,
            (icmpv6::T_DEST_UNREACH, icmpv6::UNREACH_POLICY) => Icmp6Kind::Policy,
            (icmpv6::T_DEST_UNREACH, icmpv6::UNREACH_REJECT) => Icmp6Kind::Reject,
            (icmpv6::T_PACKET_TOO_BIG, _) => Icmp6Kind::TooBig(value),
            (icmpv6::T_TIME_EXCEEDED, icmpv6::EXCEEDED_HOP_LIMIT) => Icmp6Kind::HopLimit,
            (icmpv6::T_TIME_EXCEEDED, _) => Icmp6Kind::Reassembly,
            (icmpv6::T_PARAM_PROBLEM, c) => Icmp6Kind::Param(c, value),
            (t, c) => Icmp6Kind::Other(t, c),
        }
    }
}

/// how to ping over IPv6
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ping6Opts {
    /// probes to send (0: until cancelled)
    pub count: u32,
    /// ICMPv6 data bytes (Linux 56, IOS 52 for its 100-byte packets)
    pub size: u16,
    /// None: the device's hop limit
    pub hops: Option<u8>,
    /// a probe larger than the path MTU is not fragmented ("-M do")
    pub no_frag: bool,
    /// time between probes; 0: send the next when the previous one is settled
    pub interval: Ns,
    pub timeout: Ns,
    pub src: Option<Ipv6>,
    /// the interface ("-I eth0", "fe80::1%eth0", IOS "Output Interface")
    pub oif: Option<u16>,
}

impl Default for Ping6Opts {
    fn default() -> Ping6Opts {
        Ping6Opts { count: 4, size: 56, hops: None, no_frag: false, interval: SEC, timeout: 2 * SEC, src: None, oif: None }
    }
}

/// what happened to one probe
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ping6Ev {
    /// `bytes`: the ICMPv6 message; `hops`: the hop limit it arrived with
    Reply { seq: u16, from: Ipv6, bytes: u16, hops: u8, rtt: Ns },
    Timeout { seq: u16 },
    Error { seq: u16, from: Ipv6, kind: Icmp6Kind },
}

#[derive(Clone, Debug)]
pub struct Ping6Job {
    pub dst: Ipv6,
    pub src: Ipv6,
    pub opts: Ping6Opts,
    pub ident: u16,
    pub next_seq: u16,
    pub sent: u32,
    pub received: u32,
    pub errors: u32,
    /// sequence number -> when it was sent
    pub out: BTreeMap<u16, Ns>,
    pub events: Vec<Ping6Ev>,
    pub rtt_min: Ns,
    pub rtt_max: Ns,
    pub rtt_sum: Ns,
    next_send: Ns,
}

/// one probe of a hop: who answered and how fast (None: no answer, "*")
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Trace6Probe {
    pub from: Option<Ipv6>,
    pub rtt: Ns,
    pub kind: Option<Icmp6Kind>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Trace6Hop {
    pub ttl: u8,
    pub probes: Vec<Trace6Probe>,
}

#[derive(Clone, Debug)]
pub struct Trace6Job {
    pub dst: Ipv6,
    pub src: Ipv6,
    pub oif: Option<u16>,
    pub opts: TraceOpts,
    /// the UDP source port
    pub ident: u16,
    pub ttl: u8,
    pub hops: Vec<Trace6Hop>,
    pub reached: bool,
    waiting: Option<(u16, Ns)>,
    next_port: u16,
}

fn push_probe(t: &mut Trace6Job, ttl: u8, p: Trace6Probe) {
    match t.hops.last_mut() {
        Some(h) if h.ttl == ttl => h.probes.push(p),
        _ => t.hops.push(Trace6Hop { ttl, probes: alloc::vec![p] }),
    }
}

impl Engine {
    fn new_job6(&mut self, ni: usize, kind: JobKind) -> JobId {
        self.next_job += 1;
        let id = self.next_job;
        let dev = self.dev_of(ni);
        self.jobs.insert(id, Job { id, dev, node: ni, started: self.now, done: false, kind, epoch: 0 });
        id
    }
    fn job6_tick_at(&mut self, id: JobId, at: Ns) {
        let Some(j) = self.jobs.get_mut(&id) else {
            return;
        };
        j.epoch = j.epoch.wrapping_add(1);
        let epoch = j.epoch;
        self.schedule(at, Ev::Job { job: id, epoch });
    }

    // ---------------------------------------------------------------- ping

    /// starts a ping over IPv6 from a device
    pub fn ping6(&mut self, dev: u32, dst: Ipv6, o: Ping6Opts) -> Result<JobId, ActErr> {
        self.trace.new_batch();
        let ni = self.node_index(dev).ok_or(ActErr::UnknownDevice(dev))?;
        if self.stack(ni).is_none() {
            return Err(ActErr::NoIp(dev));
        }
        let ident = self.v6_rand(ni, 65536) as u16;
        let src = o.src.or_else(|| self.ip6_source(ni, dst, o.oif)).unwrap_or(Ipv6::UNSPECIFIED);
        let routed = dst.is_multicast() || self.ip6_route(ni, dst, o.oif).is_some() || self.v6(ni).is_some_and(|v| v.find(dst).is_some());
        let now = self.now;
        let job = Ping6Job { dst, src, opts: o, ident, next_seq: 1, sent: 0, received: 0, errors: 0, out: BTreeMap::new(), events: Vec::new(), rtt_min: 0, rtt_max: 0, rtt_sum: 0, next_send: now };
        let id = self.new_job6(ni, JobKind::Ping6(job));
        if src.is_unspecified() || !routed {
            // no route: the probe never leaves
            if let Some(j) = self.jobs.get_mut(&id)
                && let JobKind::Ping6(p) = &mut j.kind
            {
                p.sent = 1;
                p.errors = 1;
                p.events.push(Ping6Ev::Error { seq: 1, from: Ipv6::UNSPECIFIED, kind: Icmp6Kind::LocalNoRoute });
                j.done = true;
            }
            return Ok(id);
        }
        self.ping6_send(id);
        self.ping6_schedule(id);
        Ok(id)
    }

    fn ping6_send(&mut self, id: JobId) {
        let now = self.now;
        let Some(j) = self.jobs.get_mut(&id) else {
            return;
        };
        let ni = j.node;
        let JobKind::Ping6(p) = &mut j.kind else {
            return;
        };
        let seq = p.next_seq;
        p.next_seq = p.next_seq.wrapping_add(1);
        p.sent += 1;
        p.out.insert(seq, now);
        p.next_send = now + p.opts.interval;
        let (src, dst, ident, size, hops, oif) = (p.src, p.dst, p.ident, p.opts.size, p.opts.hops, p.opts.oif);
        let data: Vec<u8> = (0..size as usize).map(|i| (i & 0xFF) as u8).collect();
        let out_if = oif.or_else(|| self.ip6_route(ni, dst, None).map(|nh| nh.iface));
        let hops = hops.unwrap_or_else(|| self.ip6_hops(ni, out_if));
        let mut pk = Pkt::new(HEADROOM, &data);
        Icmp6::EchoRequest { id: ident, seq }.push_v6(&mut pk, src, dst);
        Ipv6Hdr::new(src, dst, NH_ICMPV6, hops).push_onto(&mut pk);
        self.ip6_output(ni, pk.into_vec(), oif, Origin::Local { job: Some((id, seq)) }, None);
    }

    fn ping6_schedule(&mut self, id: JobId) {
        let Some(j) = self.jobs.get(&id) else {
            return;
        };
        if j.done {
            return;
        }
        let JobKind::Ping6(p) = &j.kind else {
            return;
        };
        let more = p.opts.count == 0 || p.sent < p.opts.count;
        let mut at = p.out.values().map(|t| t + p.opts.timeout).min();
        if more && p.opts.interval > 0 {
            at = Some(at.map_or(p.next_send, |a| a.min(p.next_send)));
        }
        if let Some(t) = at {
            self.job6_tick_at(id, t);
        }
    }

    fn ping6_settle(&mut self, id: JobId) {
        let now = self.now;
        let Some(j) = self.jobs.get_mut(&id) else {
            return;
        };
        let JobKind::Ping6(p) = &mut j.kind else {
            return;
        };
        let more = p.opts.count == 0 || p.sent < p.opts.count;
        if !more && p.out.is_empty() {
            j.done = true;
            j.epoch = j.epoch.wrapping_add(1);
            self.cli_job(id);
            return;
        }
        let send_now = more && ((p.opts.interval == 0 && p.out.is_empty()) || (p.opts.interval > 0 && now >= p.next_send));
        if send_now {
            self.ping6_send(id);
        }
        self.ping6_schedule(id);
        self.cli_job(id);
    }

    pub(crate) fn ping6_tick(&mut self, id: JobId) {
        let now = self.now;
        if let Some(j) = self.jobs.get_mut(&id)
            && let JobKind::Ping6(p) = &mut j.kind
        {
            let late: Vec<u16> = p.out.iter().filter(|(_, t)| **t + p.opts.timeout <= now).map(|(s, _)| *s).collect();
            for s in late {
                p.out.remove(&s);
                p.events.push(Ping6Ev::Timeout { seq: s });
            }
        }
        self.ping6_settle(id);
    }

    /// an echo reply reached node `ni`
    pub(crate) fn jobs6_echo_reply(&mut self, ni: usize, from: Ipv6, ident: u16, seq: u16, hops: u8, bytes: u16) {
        let now = self.now;
        let id = self.jobs.values().find(|j| j.node == ni && !j.done && matches!(&j.kind, JobKind::Ping6(p) if p.ident == ident)).map(|j| j.id);
        let Some(id) = id else {
            return;
        };
        let Some(j) = self.jobs.get_mut(&id) else {
            return;
        };
        let JobKind::Ping6(p) = &mut j.kind else {
            return;
        };
        let Some(sent) = p.out.remove(&seq) else {
            return; // late, a duplicate, or another member of the group
        };
        let rtt = now - sent;
        p.received += 1;
        p.rtt_sum += rtt;
        p.rtt_min = if p.received == 1 { rtt } else { p.rtt_min.min(rtt) };
        p.rtt_max = p.rtt_max.max(rtt);
        p.events.push(Ping6Ev::Reply { seq, from, bytes, hops, rtt });
        self.ping6_settle(id);
    }

    /// a ping that may not be fragmented by its sender ("-M do")
    pub(crate) fn job6_no_frag(&self, id: JobId) -> bool {
        self.jobs.get(&id).is_some_and(|j| matches!(&j.kind, JobKind::Ping6(p) if p.opts.no_frag))
    }

    /// a probe of job `id` failed before or while leaving the device (no route, neighbor
    /// discovery, too large)
    pub(crate) fn job6_error(&mut self, id: JobId, seq: u16, from: Ipv6, kind: Icmp6Kind) {
        let now = self.now;
        let Some(j) = self.jobs.get_mut(&id) else {
            return;
        };
        match &mut j.kind {
            JobKind::Ping6(p) => {
                if p.out.remove(&seq).is_some() {
                    p.errors += 1;
                    p.events.push(Ping6Ev::Error { seq, from, kind });
                    self.ping6_settle(id);
                }
            }
            JobKind::Trace6(t) => {
                if let Some((_, sent)) = t.waiting.take() {
                    let ttl = t.ttl;
                    let from = if from.is_unspecified() { None } else { Some(from) };
                    push_probe(t, ttl, Trace6Probe { from, rtt: now - sent, kind: Some(kind) });
                    t.reached = true;
                    self.trace6_next(id);
                }
            }
            _ => {}
        }
    }

    /// an ICMPv6 error about one of node `ni`'s packets (`q`: the quoted packet)
    pub(crate) fn jobs6_icmp_error(&mut self, ni: usize, from: Ipv6, typ: u8, code: u8, value: u32, q: &Ipv6View) {
        let now = self.now;
        let kind = Icmp6Kind::of(typ, code, value);
        let Some((nh, off)) = netpkt::ipv6::upper_layer(q) else {
            return;
        };
        let p = q.payload().get(off..).unwrap_or(&[]);
        if p.len() < 8 {
            return;
        }
        match nh {
            NH_ICMPV6 if p[0] == icmpv6::T_ECHO_REQUEST => {
                let (ident, seq) = (be16(p, 4), be16(p, 6));
                let id = self.jobs.values().find(|j| j.node == ni && !j.done && matches!(&j.kind, JobKind::Ping6(pj) if pj.ident == ident)).map(|j| j.id);
                if let Some(id) = id
                    && let Some(j) = self.jobs.get_mut(&id)
                    && let JobKind::Ping6(pj) = &mut j.kind
                    && pj.out.remove(&seq).is_some()
                {
                    pj.errors += 1;
                    pj.events.push(Ping6Ev::Error { seq, from, kind });
                    self.ping6_settle(id);
                }
            }
            NH_UDP => {
                let (sport, dport) = (be16(p, 0), be16(p, 2));
                let id = self
                    .jobs
                    .values()
                    .find(|j| j.node == ni && !j.done && matches!(&j.kind, JobKind::Trace6(t) if t.ident == sport && t.waiting.is_some_and(|w| w.0 == dport)))
                    .map(|j| j.id);
                if let Some(id) = id
                    && let Some(j) = self.jobs.get_mut(&id)
                    && let JobKind::Trace6(t) = &mut j.kind
                    && let Some((_, sent)) = t.waiting.take()
                {
                    let ttl = t.ttl;
                    push_probe(t, ttl, Trace6Probe { from: Some(from), rtt: now - sent, kind: Some(kind) });
                    if kind != Icmp6Kind::HopLimit {
                        t.reached = true;
                    }
                    self.trace6_next(id);
                }
            }
            _ => {}
        }
    }

    // ---------------------------------------------------------------- traceroute

    /// starts a traceroute over IPv6 from a device (`oif`: the interface of a link-local
    /// destination)
    pub fn traceroute6(&mut self, dev: u32, dst: Ipv6, o: TraceOpts, oif: Option<u16>) -> Result<JobId, ActErr> {
        self.trace.new_batch();
        let ni = self.node_index(dev).ok_or(ActErr::UnknownDevice(dev))?;
        if self.stack(ni).is_none() {
            return Err(ActErr::NoIp(dev));
        }
        let ident = 32768 + (self.v6_rand(ni, 28000)) as u16;
        let src = self.ip6_source(ni, dst, oif).unwrap_or(Ipv6::UNSPECIFIED);
        let routed = self.ip6_route(ni, dst, oif).is_some() || self.v6(ni).is_some_and(|v| v.find(dst).is_some());
        let port = o.port;
        let job = Trace6Job { dst, src, oif, opts: o, ident, ttl: 1, hops: Vec::new(), reached: false, waiting: None, next_port: port };
        let id = self.new_job6(ni, JobKind::Trace6(job));
        if src.is_unspecified() || !routed {
            if let Some(j) = self.jobs.get_mut(&id)
                && let JobKind::Trace6(t) = &mut j.kind
            {
                push_probe(t, 1, Trace6Probe { from: None, rtt: 0, kind: Some(Icmp6Kind::LocalNoRoute) });
                t.reached = true;
                j.done = true;
            }
            return Ok(id);
        }
        self.trace6_send(id);
        Ok(id)
    }

    fn trace6_send(&mut self, id: JobId) {
        let now = self.now;
        let Some(j) = self.jobs.get_mut(&id) else {
            return;
        };
        let ni = j.node;
        let JobKind::Trace6(t) = &mut j.kind else {
            return;
        };
        let dport = t.next_port;
        t.next_port = t.next_port.wrapping_add(1);
        t.waiting = Some((dport, now));
        let (src, dst, sport, ttl, size, timeout, oif) = (t.src, t.dst, t.ident, t.ttl, t.opts.size, t.opts.timeout, t.oif);
        let data: Vec<u8> = (0..size as usize).map(|i| (0x40 + (i & 0x1F)) as u8).collect();
        let mut pk = Pkt::new(HEADROOM, &data);
        UdpHdr::new(sport, dport).push_v6(&mut pk, src, dst);
        Ipv6Hdr::new(src, dst, NH_UDP, ttl).push_onto(&mut pk);
        self.ip6_output(ni, pk.into_vec(), oif, Origin::Local { job: Some((id, dport)) }, None);
        self.job6_tick_at(id, now + timeout);
    }

    fn trace6_next(&mut self, id: JobId) {
        let Some(j) = self.jobs.get_mut(&id) else {
            return;
        };
        let JobKind::Trace6(t) = &mut j.kind else {
            return;
        };
        let done_here = t.hops.last().is_some_and(|h| h.ttl == t.ttl && h.probes.len() >= t.opts.probes as usize);
        if done_here {
            if t.reached || t.ttl >= t.opts.max_hops {
                j.done = true;
                j.epoch = j.epoch.wrapping_add(1);
                self.cli_job(id);
                return;
            }
            t.ttl += 1;
        }
        self.cli_job(id);
        self.trace6_send(id);
    }

    pub(crate) fn trace6_tick(&mut self, id: JobId) {
        if let Some(j) = self.jobs.get_mut(&id)
            && let JobKind::Trace6(t) = &mut j.kind
            && t.waiting.take().is_some()
        {
            let ttl = t.ttl;
            push_probe(t, ttl, Trace6Probe { from: None, rtt: 0, kind: None });
            self.trace6_next(id);
        }
    }
}
