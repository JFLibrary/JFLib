// jobs -- things a device does over time on behalf of a command: ping and traceroute
//
//   Engine::ping(dev, dst, PingOpts) -> JobId          ICMP echo, one probe per interval
//                                                      (interval 0: the next one when the
//                                                      previous is answered or timed out, as IOS)
//   Engine::ping_host(dev, host, PingOpts) -> JobId    the same for a name: the device's
//                                                      resolver finds the address first (the
//                                                      job's `lookup`); a failed lookup ends
//                                                      the job without probes
//   Engine::traceroute(dev, dst, TraceOpts) -> JobId   UDP probes to port 33434 + n with
//                                                      TTL 1, 2, ...: Time Exceeded names the
//                                                      hops, Port Unreachable the target
//   Engine::traceroute_host(dev, host, TraceOpts)      the same for a name (looked up first, as
//                                                      ping_host does)
//   Engine::job(id) / jobs() / cancel_job(id)          the results grow while it runs
//   Engine::ping_many(k) -> started                    one probe each from k hosts at once
//                                                      (load for performance tests)
//
// The command lines (cli/print.rs) print the events in the IOS or Linux way; this module only
// measures.  It tells them when a job moved on (Engine::cli_job).  Round trip times are virtual: from handing the probe to IP (ARP included)
// until the answer is delivered.

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

use netpkt::icmp::{self, Icmp};
use netpkt::ipv4::{Ipv4Hdr, Ipv4View, PROTO_ICMP, PROTO_UDP};
use netpkt::pkt::{Pkt, HEADROOM};
use netpkt::udp::UdpHdr;
use netpkt::{be16, Ipv4};

use crate::action::ActErr;
use crate::ip::Origin;
use crate::kernel::{Engine, Ev, Ns, SEC};

pub type JobId = u32;

/// why a probe failed
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IcmpKind {
    NetUnreachable,
    HostUnreachable,
    ProtoUnreachable,
    PortUnreachable,
    FragNeeded(u16),
    Prohibited,
    TtlExceeded,
    ReasmExceeded,
    /// the sending device itself has no route
    NoRoute,
    Other(u8, u8),
}

impl IcmpKind {
    pub fn of(typ: u8, code: u8, mtu: u16) -> IcmpKind {
        match (typ, code) {
            (icmp::T_UNREACH, icmp::UNREACH_NET) => IcmpKind::NetUnreachable,
            (icmp::T_UNREACH, icmp::UNREACH_HOST) => IcmpKind::HostUnreachable,
            (icmp::T_UNREACH, icmp::UNREACH_PROTO) => IcmpKind::ProtoUnreachable,
            (icmp::T_UNREACH, icmp::UNREACH_PORT) => IcmpKind::PortUnreachable,
            (icmp::T_UNREACH, icmp::UNREACH_FRAG_NEEDED) => IcmpKind::FragNeeded(mtu),
            (icmp::T_UNREACH, 9 | 10 | icmp::UNREACH_ADMIN) => IcmpKind::Prohibited,
            (icmp::T_TIME_EXCEEDED, icmp::EXCEEDED_TTL) => IcmpKind::TtlExceeded,
            (icmp::T_TIME_EXCEEDED, _) => IcmpKind::ReasmExceeded,
            (t, c) => IcmpKind::Other(t, c),
        }
    }
}

/// how to ping
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PingOpts {
    /// probes to send (0: until cancelled)
    pub count: u32,
    /// ICMP data bytes (Linux 56, IOS 72 for its 100-byte datagrams)
    pub size: u16,
    /// None: the device's default (IOS 255, Linux 64)
    pub ttl: Option<u8>,
    pub df: bool,
    /// time between probes; 0: send the next when the previous one is settled
    pub interval: Ns,
    pub timeout: Ns,
    pub src: Option<Ipv4>,
}

impl Default for PingOpts {
    fn default() -> PingOpts {
        PingOpts { count: 4, size: 56, ttl: None, df: false, interval: SEC, timeout: 2 * SEC, src: None }
    }
}

/// what happened to one probe
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PingEv {
    Reply { seq: u16, from: Ipv4, bytes: u16, ttl: u8, rtt: Ns },
    Timeout { seq: u16 },
    Error { seq: u16, from: Ipv4, kind: IcmpKind },
}

#[derive(Clone, Debug)]
pub struct PingJob {
    pub dst: Ipv4,
    pub src: Ipv4,
    pub opts: PingOpts,
    pub ident: u16,
    pub next_seq: u16,
    pub sent: u32,
    pub received: u32,
    pub errors: u32,
    /// sequence number -> when it was sent
    pub out: BTreeMap<u16, Ns>,
    pub events: Vec<PingEv>,
    pub rtt_min: Ns,
    pub rtt_max: Ns,
    pub rtt_sum: Ns,
    next_send: Ns,
    /// the name it was started with (ping_host)
    pub host: Option<String>,
    /// the lookup of the device's resolver for the name
    pub lookup: Option<u32>,
}

/// how to trace a route
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TraceOpts {
    pub max_hops: u8,
    pub probes: u8,
    pub timeout: Ns,
    /// the first destination port
    pub port: u16,
    /// UDP data bytes
    pub size: u16,
}

impl Default for TraceOpts {
    fn default() -> TraceOpts {
        TraceOpts { max_hops: 30, probes: 3, timeout: 3 * SEC, port: 33434, size: 32 }
    }
}

/// one probe of a hop: who answered and how fast (None: no answer, "*")
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TraceProbe {
    pub from: Option<Ipv4>,
    pub rtt: Ns,
    pub kind: Option<IcmpKind>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TraceHop {
    pub ttl: u8,
    pub probes: Vec<TraceProbe>,
}

#[derive(Clone, Debug)]
pub struct TraceJob {
    pub dst: Ipv4,
    pub src: Ipv4,
    pub opts: TraceOpts,
    /// the UDP source port
    pub ident: u16,
    pub ttl: u8,
    pub hops: Vec<TraceHop>,
    pub reached: bool,
    /// the destination port of the probe on its way and when it left
    waiting: Option<(u16, Ns)>,
    next_port: u16,
    /// the name it was started with (traceroute_host)
    pub host: Option<String>,
    /// the lookup of the device's resolver for the name
    pub lookup: Option<u32>,
}

#[derive(Clone, Debug)]
pub enum JobKind {
    Ping(PingJob),
    Trace(TraceJob),
}

/// a running or finished job
#[derive(Clone, Debug)]
pub struct Job {
    pub id: JobId,
    pub dev: u32,
    pub(crate) node: usize,
    pub started: Ns,
    pub done: bool,
    pub kind: JobKind,
    pub(crate) epoch: u32,
}

impl Engine {
    fn new_job(&mut self, ni: usize, kind: JobKind) -> JobId {
        self.next_job += 1;
        let id = self.next_job;
        let dev = self.dev_of(ni);
        self.jobs.insert(id, Job { id, dev, node: ni, started: self.now, done: false, kind, epoch: 0 });
        id
    }
    fn job_tick_at(&mut self, id: JobId, at: Ns) {
        let Some(j) = self.jobs.get_mut(&id) else {
            return;
        };
        j.epoch = j.epoch.wrapping_add(1);
        let epoch = j.epoch;
        self.schedule(at, Ev::Job { job: id, epoch });
    }
    pub fn job(&self, id: JobId) -> Option<&Job> {
        self.jobs.get(&id)
    }
    pub fn jobs(&self) -> impl Iterator<Item = &Job> {
        self.jobs.values()
    }
    /// stops a job (Ctrl+C); false when it is unknown or finished
    pub fn cancel_job(&mut self, id: JobId) -> bool {
        match self.jobs.get_mut(&id) {
            Some(j) if !j.done => {
                j.done = true;
                j.epoch = j.epoch.wrapping_add(1);
                true
            }
            _ => false,
        }
    }
    /// the jobs of a node end (power off, removed)
    pub(crate) fn end_jobs_of(&mut self, ni: usize) {
        for j in self.jobs.values_mut() {
            if j.node == ni {
                j.done = true;
                j.epoch = j.epoch.wrapping_add(1);
            }
        }
    }

    // ---------------------------------------------------------------- ping

    /// starts a ping from a device
    pub fn ping(&mut self, dev: u32, dst: Ipv4, o: PingOpts) -> Result<JobId, ActErr> {
        self.trace.new_batch();
        let ni = self.node_index(dev).ok_or(ActErr::UnknownDevice(dev))?;
        if self.stack(ni).is_none() {
            return Err(ActErr::NoIp(dev));
        }
        let ident = match self.nodes.get_mut(ni) {
            Some(Some(n)) => n.rng.next_u32() as u16,
            _ => 0,
        };
        let src = o.src.or_else(|| self.source_for(ni, dst)).unwrap_or(Ipv4::UNSPECIFIED);
        let now = self.now;
        let job = PingJob {
            dst,
            src,
            opts: o,
            ident,
            next_seq: 1,
            sent: 0,
            received: 0,
            errors: 0,
            out: BTreeMap::new(),
            events: Vec::new(),
            rtt_min: 0,
            rtt_max: 0,
            rtt_sum: 0,
            next_send: now,
            host: None,
            lookup: None,
        };
        let id = self.new_job(ni, JobKind::Ping(job));
        if src.is_unspecified() {
            // no route: the probe never leaves (Linux: "Network is unreachable")
            if let Some(j) = self.jobs.get_mut(&id)
                && let JobKind::Ping(p) = &mut j.kind
            {
                p.sent = 1;
                p.errors = 1;
                p.events.push(PingEv::Error { seq: 1, from: Ipv4::UNSPECIFIED, kind: IcmpKind::NoRoute });
                j.done = true;
            }
            return Ok(id);
        }
        self.ping_send(id);
        self.ping_schedule(id);
        Ok(id)
    }

    /// pings a name or an address; a name is looked up by the device's resolver first
    pub fn ping_host(&mut self, dev: u32, host: &str, o: PingOpts) -> Result<JobId, ActErr> {
        if let Some(ip) = Ipv4::parse(host.trim()) {
            return self.ping(dev, ip, o);
        }
        let ni = self.node_index(dev).ok_or(ActErr::UnknownDevice(dev))?;
        if self.stack(ni).is_none() || self.os(ni).is_none() {
            return Err(ActErr::NoIp(dev));
        }
        let ident = match self.nodes.get_mut(ni) {
            Some(Some(n)) => n.rng.next_u32() as u16,
            _ => 0,
        };
        let job = PingJob {
            dst: Ipv4::UNSPECIFIED,
            src: Ipv4::UNSPECIFIED,
            opts: o,
            ident,
            next_seq: 1,
            sent: 0,
            received: 0,
            errors: 0,
            out: BTreeMap::new(),
            events: Vec::new(),
            rtt_min: 0,
            rtt_max: 0,
            rtt_sum: 0,
            next_send: Ns::MAX,
            host: Some(String::from(host.trim())),
            lookup: None,
        };
        let id = self.new_job(ni, JobKind::Ping(job));
        let lookup = self.lookup_start(ni, host, netpkt::dns::TYPE_A, crate::resolver::Owner::Ping(id));
        if let Some(j) = self.jobs.get_mut(&id)
            && let JobKind::Ping(p) = &mut j.kind
        {
            p.lookup = Some(lookup);
        }
        Ok(id)
    }

    /// the lookup of a ping_host job is done: ping the address, or end without probes
    pub(crate) fn ping_resolved(&mut self, id: JobId, addr: Option<Ipv4>) {
        let now = self.now;
        let Some(j) = self.jobs.get(&id) else {
            return;
        };
        if j.done {
            return;
        }
        let ni = j.node;
        let Some(dst) = addr else {
            if let Some(j) = self.jobs.get_mut(&id) {
                j.done = true;
                j.epoch = j.epoch.wrapping_add(1);
            }
            // a console waits for it ("ping: NAME: Name or service not known")
            self.cli_job(id);
            return;
        };
        let src = match &j.kind {
            JobKind::Ping(p) => p.opts.src,
            _ => return,
        }
        .or_else(|| self.source_for(ni, dst))
        .unwrap_or(Ipv4::UNSPECIFIED);
        let Some(j) = self.jobs.get_mut(&id) else {
            return;
        };
        let JobKind::Ping(p) = &mut j.kind else {
            return;
        };
        p.dst = dst;
        p.src = src;
        p.next_send = now;
        if src.is_unspecified() {
            p.sent = 1;
            p.errors = 1;
            p.events.push(PingEv::Error { seq: 1, from: Ipv4::UNSPECIFIED, kind: IcmpKind::NoRoute });
            j.done = true;
            self.cli_job(id);
            return;
        }
        self.ping_send(id);
        self.ping_schedule(id);
        // the console prints its first line now ("PING NAME (ADDR) ...")
        self.cli_job(id);
    }

    fn ping_send(&mut self, id: JobId) {
        let now = self.now;
        let Some(j) = self.jobs.get_mut(&id) else {
            return;
        };
        let ni = j.node;
        let JobKind::Ping(p) = &mut j.kind else {
            return;
        };
        let seq = p.next_seq;
        p.next_seq = p.next_seq.wrapping_add(1);
        p.sent += 1;
        p.out.insert(seq, now);
        p.next_send = now + p.opts.interval;
        let (src, dst, ident, size, ttl, df) = (p.src, p.dst, p.ident, p.opts.size, p.opts.ttl, p.opts.df);
        let data: Vec<u8> = (0..size as usize).map(|i| (i & 0xFF) as u8).collect();
        let Some(st) = self.stack_mut(ni) else {
            return;
        };
        let ttl = ttl.unwrap_or(st.ttl);
        let ipid = st.next_id();
        let mut pk = Pkt::new(HEADROOM, &data);
        Icmp::EchoRequest { id: ident, seq }.push_onto(&mut pk);
        let mut h = Ipv4Hdr::new(src, dst, PROTO_ICMP, ttl);
        h.id = ipid;
        h.df = df;
        h.push_onto(&mut pk);
        self.ip_output(ni, pk.into_vec(), Origin::Local { job: Some((id, seq)) }, None);
    }

    /// the next tick: the next send or the earliest timeout
    fn ping_schedule(&mut self, id: JobId) {
        let Some(j) = self.jobs.get(&id) else {
            return;
        };
        if j.done {
            return;
        }
        let JobKind::Ping(p) = &j.kind else {
            return;
        };
        let more = p.opts.count == 0 || p.sent < p.opts.count;
        let mut at = p.out.values().map(|t| t + p.opts.timeout).min();
        if more && p.opts.interval > 0 {
            at = Some(at.map_or(p.next_send, |a| a.min(p.next_send)));
        }
        if let Some(t) = at {
            self.job_tick_at(id, t);
        }
    }

    /// a probe got its answer or failed: settle it, maybe send the next, maybe finish
    fn ping_settle(&mut self, id: JobId) {
        let now = self.now;
        let Some(j) = self.jobs.get_mut(&id) else {
            return;
        };
        let JobKind::Ping(p) = &mut j.kind else {
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
            self.ping_send(id);
        }
        self.ping_schedule(id);
        self.cli_job(id);
    }

    fn ping_tick(&mut self, id: JobId) {
        let now = self.now;
        if let Some(j) = self.jobs.get_mut(&id)
            && let JobKind::Ping(p) = &mut j.kind
        {
            let late: Vec<u16> = p.out.iter().filter(|(_, t)| **t + p.opts.timeout <= now).map(|(s, _)| *s).collect();
            for s in late {
                p.out.remove(&s);
                p.events.push(PingEv::Timeout { seq: s });
            }
        }
        self.ping_settle(id);
    }

    /// an echo reply reached node `ni`
    pub(crate) fn jobs_echo_reply(&mut self, ni: usize, from: Ipv4, ident: u16, seq: u16, ttl: u8, bytes: u16) {
        let now = self.now;
        let id = self.jobs.values().find(|j| j.node == ni && !j.done && matches!(&j.kind, JobKind::Ping(p) if p.ident == ident)).map(|j| j.id);
        let Some(id) = id else {
            return;
        };
        let Some(j) = self.jobs.get_mut(&id) else {
            return;
        };
        let JobKind::Ping(p) = &mut j.kind else {
            return;
        };
        let Some(sent) = p.out.remove(&seq) else {
            return; // late or duplicate
        };
        let rtt = now - sent;
        p.received += 1;
        p.rtt_sum += rtt;
        p.rtt_min = if p.received == 1 { rtt } else { p.rtt_min.min(rtt) };
        p.rtt_max = p.rtt_max.max(rtt);
        p.events.push(PingEv::Reply { seq, from, bytes, ttl, rtt });
        self.ping_settle(id);
    }

    /// a probe of job `id` failed before leaving the device (no route, ARP, too large)
    pub(crate) fn job_error(&mut self, id: JobId, seq: u16, from: Ipv4, kind: IcmpKind) {
        let now = self.now;
        let Some(j) = self.jobs.get_mut(&id) else {
            return;
        };
        match &mut j.kind {
            JobKind::Ping(p) => {
                if p.out.remove(&seq).is_some() {
                    p.errors += 1;
                    p.events.push(PingEv::Error { seq, from, kind });
                    self.ping_settle(id);
                }
            }
            JobKind::Trace(t) => {
                if let Some((_, sent)) = t.waiting.take() {
                    let ttl = t.ttl;
                    push_probe(t, ttl, TraceProbe { from: Some(from), rtt: now - sent, kind: Some(kind) });
                    t.reached = true;
                    self.trace_next(id);
                }
            }
        }
    }

    /// an ICMP error about one of node `ni`'s packets (`q` is the quoted packet)
    pub(crate) fn jobs_icmp_error(&mut self, ni: usize, from: Ipv4, typ: u8, code: u8, mtu: u16, q: &Ipv4View) {
        let now = self.now;
        let kind = IcmpKind::of(typ, code, mtu);
        let p = q.payload();
        if p.len() < 8 {
            return;
        }
        match q.proto() {
            PROTO_ICMP if p[0] == icmp::T_ECHO_REQUEST => {
                let (ident, seq) = (be16(p, 4), be16(p, 6));
                let id = self.jobs.values().find(|j| j.node == ni && !j.done && matches!(&j.kind, JobKind::Ping(pj) if pj.ident == ident)).map(|j| j.id);
                if let Some(id) = id
                    && let Some(j) = self.jobs.get_mut(&id)
                    && let JobKind::Ping(pj) = &mut j.kind
                    && pj.out.remove(&seq).is_some()
                {
                    pj.errors += 1;
                    pj.events.push(PingEv::Error { seq, from, kind });
                    self.ping_settle(id);
                }
            }
            PROTO_UDP => {
                let (sport, dport) = (be16(p, 0), be16(p, 2));
                let id = self
                    .jobs
                    .values()
                    .find(|j| j.node == ni && !j.done && matches!(&j.kind, JobKind::Trace(t) if t.ident == sport && t.waiting.is_some_and(|w| w.0 == dport)))
                    .map(|j| j.id);
                if let Some(id) = id
                    && let Some(j) = self.jobs.get_mut(&id)
                    && let JobKind::Trace(t) = &mut j.kind
                    && let Some((_, sent)) = t.waiting.take()
                {
                    let ttl = t.ttl;
                    push_probe(t, ttl, TraceProbe { from: Some(from), rtt: now - sent, kind: Some(kind) });
                    if kind != IcmpKind::TtlExceeded {
                        t.reached = true;
                    }
                    self.trace_next(id);
                }
            }
            _ => {}
        }
    }

    // ---------------------------------------------------------------- traceroute

    /// starts a traceroute from a device
    pub fn traceroute(&mut self, dev: u32, dst: Ipv4, o: TraceOpts) -> Result<JobId, ActErr> {
        self.trace.new_batch();
        let ni = self.node_index(dev).ok_or(ActErr::UnknownDevice(dev))?;
        if self.stack(ni).is_none() {
            return Err(ActErr::NoIp(dev));
        }
        let ident = match self.nodes.get_mut(ni) {
            Some(Some(n)) => 32768 + (n.rng.next_u32() % 28000) as u16,
            _ => 40000,
        };
        let src = self.source_for(ni, dst).unwrap_or(Ipv4::UNSPECIFIED);
        let port = o.port;
        let job = TraceJob { dst, src, opts: o, ident, ttl: 1, hops: Vec::new(), reached: false, waiting: None, next_port: port, host: None, lookup: None };
        let id = self.new_job(ni, JobKind::Trace(job));
        if src.is_unspecified() {
            if let Some(j) = self.jobs.get_mut(&id)
                && let JobKind::Trace(t) = &mut j.kind
            {
                push_probe(t, 1, TraceProbe { from: None, rtt: 0, kind: Some(IcmpKind::NoRoute) });
                t.reached = true;
                j.done = true;
            }
            return Ok(id);
        }
        self.trace_send(id);
        Ok(id)
    }

    /// traces the route to a name or an address; a name is looked up by the device's resolver
    /// first (a failed lookup ends the job without probes)
    pub fn traceroute_host(&mut self, dev: u32, host: &str, o: TraceOpts) -> Result<JobId, ActErr> {
        if let Some(ip) = Ipv4::parse(host.trim()) {
            return self.traceroute(dev, ip, o);
        }
        self.trace.new_batch();
        let ni = self.node_index(dev).ok_or(ActErr::UnknownDevice(dev))?;
        if self.stack(ni).is_none() || self.os(ni).is_none() {
            return Err(ActErr::NoIp(dev));
        }
        let ident = match self.nodes.get_mut(ni) {
            Some(Some(n)) => 32768 + (n.rng.next_u32() % 28000) as u16,
            _ => 40000,
        };
        let port = o.port;
        let job = TraceJob { dst: Ipv4::UNSPECIFIED, src: Ipv4::UNSPECIFIED, opts: o, ident, ttl: 1, hops: Vec::new(), reached: false, waiting: None, next_port: port, host: Some(String::from(host.trim())), lookup: None };
        let id = self.new_job(ni, JobKind::Trace(job));
        let lookup = self.lookup_start(ni, host, netpkt::dns::TYPE_A, crate::resolver::Owner::Trace(id));
        if let Some(j) = self.jobs.get_mut(&id)
            && let JobKind::Trace(t) = &mut j.kind
        {
            t.lookup = Some(lookup);
        }
        Ok(id)
    }

    /// the lookup of a traceroute_host job is done: trace the address, or end without probes
    pub(crate) fn trace_resolved(&mut self, id: JobId, addr: Option<Ipv4>) {
        let Some(j) = self.jobs.get(&id) else {
            return;
        };
        if j.done {
            return;
        }
        let ni = j.node;
        let src = addr.and_then(|dst| self.source_for(ni, dst));
        let Some(j) = self.jobs.get_mut(&id) else {
            return;
        };
        let JobKind::Trace(t) = &mut j.kind else {
            return;
        };
        match (addr, src) {
            (Some(dst), Some(src)) => {
                t.dst = dst;
                t.src = src;
                self.trace_send(id);
            }
            (Some(dst), None) => {
                // no route: the first hop fails at once, as traceroute() does it
                t.dst = dst;
                push_probe(t, 1, TraceProbe { from: None, rtt: 0, kind: Some(IcmpKind::NoRoute) });
                t.reached = true;
                j.done = true;
                j.epoch = j.epoch.wrapping_add(1);
            }
            (None, _) => {
                j.done = true;
                j.epoch = j.epoch.wrapping_add(1);
            }
        }
        self.cli_job(id);
    }

    fn trace_send(&mut self, id: JobId) {
        let now = self.now;
        let Some(j) = self.jobs.get_mut(&id) else {
            return;
        };
        let ni = j.node;
        let JobKind::Trace(t) = &mut j.kind else {
            return;
        };
        let dport = t.next_port;
        t.next_port = t.next_port.wrapping_add(1);
        t.waiting = Some((dport, now));
        let (src, dst, sport, ttl, size, timeout) = (t.src, t.dst, t.ident, t.ttl, t.opts.size, t.opts.timeout);
        let data: Vec<u8> = (0..size as usize).map(|i| (0x40 + (i & 0x1F)) as u8).collect();
        let Some(st) = self.stack_mut(ni) else {
            return;
        };
        let ipid = st.next_id();
        let mut pk = Pkt::new(HEADROOM, &data);
        UdpHdr::new(sport, dport).push_v4(&mut pk, src, dst);
        let mut h = Ipv4Hdr::new(src, dst, PROTO_UDP, ttl);
        h.id = ipid;
        h.push_onto(&mut pk);
        self.ip_output(ni, pk.into_vec(), Origin::Local { job: Some((id, dport)) }, None);
        self.job_tick_at(id, now + timeout);
    }

    /// the probe is settled: the next probe of this hop, the next hop, or the end
    fn trace_next(&mut self, id: JobId) {
        let Some(j) = self.jobs.get_mut(&id) else {
            return;
        };
        let JobKind::Trace(t) = &mut j.kind else {
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
        self.trace_send(id);
    }

    fn trace_tick(&mut self, id: JobId) {
        if let Some(j) = self.jobs.get_mut(&id)
            && let JobKind::Trace(t) = &mut j.kind
            && t.waiting.take().is_some()
        {
            let ttl = t.ttl;
            push_probe(t, ttl, TraceProbe { from: None, rtt: 0, kind: None });
            self.trace_next(id);
        }
    }

    pub(crate) fn job_tick(&mut self, id: JobId, epoch: u32) {
        let Some(j) = self.jobs.get(&id) else {
            return;
        };
        if j.done || j.epoch != epoch {
            return;
        }
        match j.kind {
            JobKind::Ping(_) => self.ping_tick(id),
            JobKind::Trace(_) => self.trace_tick(id),
        }
    }
}

fn push_probe(t: &mut TraceJob, ttl: u8, p: TraceProbe) {
    match t.hops.last_mut() {
        Some(h) if h.ttl == ttl => h.probes.push(p),
        _ => t.hops.push(TraceHop { ttl, probes: alloc::vec![p] }),
    }
}

// ---------------------------------------------------------------- load for tests

impl Engine {
    /// pings from k hosts at once, one probe each (load for performance tests, --traffic): the
    /// Linux hosts with an address on an interface that is up, spread evenly in id order, each
    /// to the host half way round that list -> the pings started (their first frames leave now)
    pub fn ping_many(&mut self, k: usize) -> usize {
        let mut hosts: Vec<(u32, Ipv4)> = Vec::new();
        for (&dev, &ni) in &self.dev_idx {
            let Some(Some(n)) = self.nodes.get(ni) else {
                continue;
            };
            if !n.power || !n.model.has(jfn_model::catalog::F_HOST) || n.model.cli != jfn_model::catalog::CliStyle::Linux {
                continue;
            }
            let Some(st) = n.ip.as_ref() else {
                continue;
            };
            let up = st.ifaces.iter().enumerate().filter(|(i, f)| !f.hidden && !f.is_loopback() && self.iface_up(ni, *i as u16));
            if let Some(a) = up.into_iter().find_map(|(_, f)| f.primary()) {
                hosts.push((dev, a));
            }
        }
        let m = hosts.len();
        if m < 2 {
            return 0;
        }
        let k = k.min(m);
        let mut started = 0;
        for j in 0..k {
            let s = j * m / k;
            let (dev, dst) = (hosts[s].0, hosts[(s + m / 2) % m].1);
            if self.ping(dev, dst, PingOpts { count: 1, ..PingOpts::default() }).is_ok() {
                started += 1;
            }
        }
        started
    }
}
