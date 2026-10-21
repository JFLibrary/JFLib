// resolver -- name lookups of a device: the equivalent of /etc/resolv.conf with a cache, as
// systemd-resolved keeps one on Linux and IOS keeps its host cache
//
//   ResolvConf { servers, search, ndots, timeout, attempts }   what a device asks and how
//   Resolver                 the static settings (resolv.conf, "ip name-server", "ip
//                            domain-name") and the ones from DHCP; static ones win
//   Engine::lookup(dev, name, qtype) -> lookup id              starts a lookup
//   Engine::lookup_state(dev, id) -> Option<&Lookup>           progress and result: every
//                            question to every server (nslookup, dig), the last reply
//   Engine::resolver_cache(dev) -> Vec<CacheRow>               "show hosts", resolvectl
//   Engine::resolv_conf(dev) -> Option<ResolvConf>             "cat /etc/resolv.conf"
//   Engine::resolver_latency(dev) -> LatencyView               lookups of the last 5 minutes:
//                            count, p50, p95, p99 (a base::metrics::Window, 300 x 1 s)
//
// A lookup takes an address literal as it is, then the host table ("localhost", static
// entries), then asks the DNS: a name with fewer dots than ndots goes through the search
// domains first, any other name is tried as it is first (glibc).  NXDOMAIN or an answer
// without data moves on to the next candidate; a server that does not answer within the
// timeout passes the question to the next server, every server `attempts` times, and when
// nobody answers at all the lookup ends (glibc's TRY_AGAIN); SERVFAIL and REFUSED ask the next
// server, and when all of them failed, the next candidate.  Answers are cached for the
// smallest TTL of their records, negative answers for the SOA minimum (RFC 2308), at most an
// hour.  Linux waits 5 s for an answer and tries twice; IOS waits 3 s and tries twice.  Query
// ids and source ports come from the device's random numbers.

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

use base::metrics::Window;

use netpkt::dns::{self, Message, RData, Rr, CLASS_IN, TYPE_A, TYPE_CNAME, TYPE_SOA};
use netpkt::udp::PORT_DNS;
use netpkt::Ipv4;

use crate::action::ActErr;
use crate::kernel::{Engine, Ns, SEC};
use crate::os::{Dgram, Job, Owner as SockOwner, SvcEvent, SLOT_RESOLVER};
use crate::trace::Note;

/// cached entries at most
pub const CACHE_MAX: usize = 4096;
/// a negative answer is kept at most this long (s)
pub const NEG_TTL_MAX: u32 = 3600;
/// CNAMEs followed at most
const CNAME_DEPTH: usize = 8;

/// how a device resolves names
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvConf {
    /// name servers, at most three are asked
    pub servers: Vec<Ipv4>,
    /// search domains (resolv.conf "search", IOS "ip domain-name")
    pub search: Vec<String>,
    /// names with fewer dots go through the search list first
    pub ndots: u8,
    /// waiting for one server
    pub timeout: Ns,
    /// rounds over all servers
    pub attempts: u8,
}

/// who started a lookup
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Owner {
    /// a command (nslookup, dig, ping)
    Cli,
    Svc(u32),
    /// a ping job waiting for its address
    Ping(u32),
    /// a traceroute job waiting for its address
    Trace(u32),
}

/// why a lookup failed
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LookupErr {
    /// NXDOMAIN for every candidate
    NotFound,
    /// the name exists without a record of the type
    NoData,
    /// no server answered
    Timeout,
    ServFail,
    Refused,
    /// no name server is set
    NoServer,
    BadName,
}

impl LookupErr {
    /// German words for notes (the app translates them)
    pub fn text(self) -> &'static str {
        // i18n
        match self {
            LookupErr::NotFound => "unbekannter Name",
            LookupErr::NoData => "kein passender Eintrag",
            LookupErr::Timeout => "kein DNS-Server antwortet",
            LookupErr::ServFail => "Fehler beim DNS-Server",
            LookupErr::Refused => "der DNS-Server lehnt ab",
            LookupErr::NoServer => "kein DNS-Server eingetragen",
            LookupErr::BadName => "ungültiger Name",
        }
    }
}

/// where an answer came from
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Literal,
    Hosts,
    Cache,
    Server(Ipv4),
}

/// a successful lookup
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Answer {
    /// the candidate that was answered ("www.lab" for "www")
    pub name: String,
    /// the end of the CNAME chain
    pub canonical: String,
    pub addrs: Vec<Ipv4>,
    /// the answer section
    pub records: Vec<Rr>,
    pub from: Source,
    /// seconds it stays valid
    pub ttl: u32,
}

/// what became of one question to one server
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TryOut {
    Answer,
    NxDomain,
    NoData,
    ServFail,
    Refused,
    Timeout,
    /// the device has no route to the server
    NoRoute,
}

/// one question to one server
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Try {
    pub name: String,
    pub server: Ipv4,
    pub sent: Ns,
    /// None while waiting
    pub outcome: Option<TryOut>,
    pub rtt: Ns,
}

/// a lookup in progress or done
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Lookup {
    pub id: u32,
    pub name: String,
    pub qtype: u16,
    pub owner: Owner,
    pub started: Ns,
    pub done: Option<Ns>,
    pub result: Option<Result<Answer, LookupErr>>,
    pub tries: Vec<Try>,
    /// the last DNS reply (dig shows it)
    pub reply: Option<Message>,
    cands: Vec<String>,
    cand: usize,
    servers: Vec<Ipv4>,
    server: usize,
    round: u8,
    qid: u16,
    port: u16,
    deadline: Option<Ns>,
    last_err: LookupErr,
    /// the error of the name as it is, when it was asked first (dots >= ndots)
    asis_err: Option<LookupErr>,
    nodata: bool,
}

impl Lookup {
    /// the current candidate does not exist (nx) or has no data: on to the next
    fn negative(&mut self, nx: bool) {
        let e = if nx { LookupErr::NotFound } else { LookupErr::NoData };
        if self.cand == 0 && self.cands.first().is_some_and(|c| dns::name_eq(c, &self.name)) && self.cands.len() > 1 {
            self.asis_err = Some(e);
        }
        self.nodata |= !nx;
        self.last_err = e;
        self.cand += 1;
    }
}

/// a cached answer (records empty and rcode 0: no data)
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CacheEntry {
    pub at: Ns,
    pub ttl: u32,
    pub rcode: u8,
    pub records: Vec<Rr>,
}

/// a cache entry as "show hosts" lists it
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CacheRow {
    pub name: String,
    pub qtype: u16,
    /// seconds left
    pub ttl: u32,
    /// NXDOMAIN or no data
    pub negative: bool,
    pub records: Vec<Rr>,
}

/// the resolver of a device
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resolver {
    /// set by hand (servers and search domains; empty: those from DHCP)
    pub statics: ResolvConf,
    /// learned from DHCP
    pub dhcp: Option<(Vec<Ipv4>, Vec<String>)>,
    pub cache_on: bool,
    pub(crate) cache: BTreeMap<(String, u16), CacheEntry>,
    pub(crate) lookups: BTreeMap<u32, Lookup>,
    next: u32,
    /// how long lookups took (ns), five minutes in seconds
    pub latency: Window,
}

/// the lookups of the last five minutes
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LatencyView {
    pub count: u64,
    /// ns
    pub p50: u64,
    pub p95: u64,
    pub p99: u64,
    pub max: u64,
}

impl Resolver {
    pub fn new(ios: bool) -> Resolver {
        let statics = ResolvConf { servers: Vec::new(), search: Vec::new(), ndots: 1, timeout: if ios { 3 * SEC } else { 5 * SEC }, attempts: 2 };
        Resolver { statics, dhcp: None, cache_on: true, cache: BTreeMap::new(), lookups: BTreeMap::new(), next: 0, latency: Window::new(300, SEC) }
    }
    /// what is used now: static servers and search domains, else those from DHCP
    pub fn conf(&self) -> ResolvConf {
        let mut c = self.statics.clone();
        if let Some((s, d)) = &self.dhcp {
            if c.servers.is_empty() {
                c.servers = s.clone();
            }
            if c.search.is_empty() {
                c.search = d.clone();
            }
        }
        c.servers.truncate(3);
        c
    }
    pub(crate) fn set_dhcp(&mut self, d: Option<(Vec<Ipv4>, Vec<String>)>) {
        self.dhcp = d;
    }
    /// forgets the cache
    pub fn flush(&mut self) {
        self.cache.clear();
    }
    /// the power went off: lookups end, the cache is gone
    pub(crate) fn power_off(&mut self, now: Ns) {
        for l in self.lookups.values_mut() {
            if l.done.is_none() {
                l.deadline = None;
                l.done = Some(now);
                l.result = Some(Err(LookupErr::Timeout));
            }
        }
        self.cache.clear();
    }
    fn cached(&self, now: Ns, name: &str, qtype: u16) -> Option<(&CacheEntry, u32)> {
        let e = self.cache.get(&(name.to_ascii_lowercase(), qtype))?;
        let age = (now.saturating_sub(e.at) / SEC) as u32;
        if age >= e.ttl { None } else { Some((e, e.ttl - age)) }
    }
}

/// the candidates for a name (glibc order)
fn candidates(name: &str, c: &ResolvConf) -> Vec<String> {
    if let Some(abs) = name.strip_suffix('.') {
        return alloc::vec![String::from(abs)];
    }
    let dots = name.bytes().filter(|&b| b == b'.').count();
    let searched: Vec<String> = c.search.iter().filter(|d| !d.is_empty()).map(|d| alloc::format!("{}.{}", name, d.trim_end_matches('.'))).collect();
    let mut v = Vec::new();
    if dots >= c.ndots as usize {
        v.push(String::from(name));
        v.extend(searched);
    } else {
        v.extend(searched);
        v.push(String::from(name));
    }
    v
}

/// the CNAME chain from `name` and the records of `qtype` at its end
fn follow(records: &[Rr], name: &str, qtype: u16) -> (String, Vec<Rr>, u32) {
    let mut canon = String::from(name);
    let mut ttl = u32::MAX;
    for _ in 0..CNAME_DEPTH {
        match records.iter().find(|r| r.rtype == TYPE_CNAME && dns::name_eq(&r.name, &canon)) {
            Some(r) if qtype != TYPE_CNAME => {
                ttl = ttl.min(r.ttl);
                if let RData::Cname(t) = &r.data {
                    canon = t.clone();
                }
            }
            _ => break,
        }
    }
    let hits: Vec<Rr> = records.iter().filter(|r| r.rtype == qtype && r.class == CLASS_IN && dns::name_eq(&r.name, &canon)).cloned().collect();
    for r in &hits {
        ttl = ttl.min(r.ttl);
    }
    (canon, hits, if ttl == u32::MAX { 0 } else { ttl })
}

/// the TTL of a negative answer: SOA minimum and the SOA's own TTL (RFC 2308 5)
fn neg_ttl(m: &Message) -> u32 {
    m.authority
        .iter()
        .find(|r| r.rtype == TYPE_SOA)
        .and_then(|r| match &r.data {
            RData::Soa(s) => Some(s.minimum.min(r.ttl)),
            _ => None,
        })
        .unwrap_or(0)
        .min(NEG_TTL_MAX)
}

impl Engine {
    /// starts a lookup on a device (nslookup, dig, ping by name)
    pub fn lookup(&mut self, dev: u32, name: &str, qtype: u16) -> Result<u32, ActErr> {
        let ni = self.node_index(dev).ok_or(ActErr::UnknownDevice(dev))?;
        if self.os(ni).is_none() {
            return Err(ActErr::NoIp(dev));
        }
        Ok(self.lookup_start(ni, name, qtype, Owner::Cli))
    }

    /// a lookup of a device
    pub fn lookup_state(&self, dev: u32, id: u32) -> Option<&Lookup> {
        self.os(self.node_index(dev)?)?.resolver.lookups.get(&id)
    }

    /// what the resolver of a device keeps (expired entries left out)
    pub fn resolver_cache(&self, dev: u32) -> Vec<CacheRow> {
        let now = self.now;
        let Some(os) = self.node_index(dev).and_then(|ni| self.os(ni)) else {
            return Vec::new();
        };
        os.resolver
            .cache
            .iter()
            .filter_map(|((name, qtype), e)| {
                let age = (now.saturating_sub(e.at) / SEC) as u32;
                (age < e.ttl).then(|| CacheRow { name: name.clone(), qtype: *qtype, ttl: e.ttl - age, negative: e.rcode != 0 || e.records.is_empty(), records: e.records.clone() })
            })
            .collect()
    }

    /// the resolver settings in use
    pub fn resolv_conf(&self, dev: u32) -> Option<ResolvConf> {
        Some(self.os(self.node_index(dev)?)?.resolver.conf())
    }

    /// how long the lookups of the last five minutes took
    pub fn resolver_latency(&self, dev: u32) -> LatencyView {
        let now = self.now;
        let Some(w) = self.node_index(dev).and_then(|ni| self.os(ni)).map(|o| &o.resolver.latency) else {
            return LatencyView::default();
        };
        let h = w.hist(now);
        LatencyView { count: h.count(), p50: h.p50().unwrap_or(0), p95: h.p95().unwrap_or(0), p99: h.p99().unwrap_or(0), max: h.max().unwrap_or(0) }
    }

    pub(crate) fn lookup_start(&mut self, ni: usize, name: &str, qtype: u16, owner: Owner) -> u32 {
        let now = self.now;
        let name = String::from(name.trim());
        let Some(os) = self.os_mut(ni) else {
            return 0;
        };
        os.resolver.next += 1;
        let id = os.resolver.next;
        let conf = os.resolver.conf();
        let l = Lookup {
            id,
            name: name.clone(),
            qtype,
            owner,
            started: now,
            done: None,
            result: None,
            tries: Vec::new(),
            reply: None,
            cands: candidates(&name, &conf),
            cand: 0,
            servers: conf.servers.clone(),
            server: 0,
            round: 0,
            qid: 0,
            port: 0,
            deadline: None,
            last_err: LookupErr::NotFound,
            asis_err: None,
            nodata: false,
        };
        // a literal, the host table
        let literal = if qtype == TYPE_A { Ipv4::parse(&name) } else { None };
        let bare = name.trim_end_matches('.');
        let hosted = if qtype != TYPE_A {
            None
        } else if bare.eq_ignore_ascii_case("localhost") {
            Some(Ipv4::LOCALHOST)
        } else {
            os.hosts.iter().find(|(h, _)| h.eq_ignore_ascii_case(bare)).map(|h| h.1)
        };
        os.resolver.lookups.insert(id, l);
        if let Some(ip) = literal {
            self.lookup_finish(ni, id, Ok(Answer { name: name.clone(), canonical: name, addrs: alloc::vec![ip], records: Vec::new(), from: Source::Literal, ttl: 0 }));
            return id;
        }
        if let Some(ip) = hosted {
            self.lookup_finish(ni, id, Ok(Answer { name: String::from(bare), canonical: String::from(bare), addrs: alloc::vec![ip], records: Vec::new(), from: Source::Hosts, ttl: 0 }));
            return id;
        }
        if dns::labels(&name).is_none() || name.is_empty() {
            self.lookup_finish(ni, id, Err(LookupErr::BadName));
            return id;
        }
        self.lookup_next(ni, id);
        id
    }

    fn lookup_mut(&mut self, ni: usize, id: u32) -> Option<&mut Lookup> {
        self.os_mut(ni)?.resolver.lookups.get_mut(&id)
    }

    /// the current candidate: from the cache, or ask the first server
    fn lookup_next(&mut self, ni: usize, id: u32) {
        let now = self.now;
        let dev = self.dev_of(ni);
        loop {
            let Some(os) = self.os(ni) else {
                return;
            };
            let Some(l) = os.resolver.lookups.get(&id) else {
                return;
            };
            if l.cand >= l.cands.len() {
                // glibc: the error of the name as it is, else NODATA, else the last one
                let e = l.asis_err.unwrap_or(if l.nodata { LookupErr::NoData } else { l.last_err });
                self.lookup_finish(ni, id, Err(e));
                return;
            }
            let cand = l.cands[l.cand].clone();
            let qtype = l.qtype;
            let hit = if os.resolver.cache_on { os.resolver.cached(now, &cand, qtype).map(|(e, left)| (e.clone(), left)) } else { None };
            match hit {
                Some((e, left)) if e.rcode == 0 && !e.records.is_empty() => {
                    let (canon, hits, _) = follow(&e.records, &cand, qtype);
                    let addrs = hits.iter().filter_map(|r| if let RData::A(a) = r.data { Some(a) } else { None }).collect();
                    self.lookup_finish(ni, id, Ok(Answer { name: cand.clone(), canonical: canon, addrs, records: e.records.clone(), from: Source::Cache, ttl: left }));
                    return;
                }
                Some((e, _)) => {
                    let nx = e.rcode == dns::RCODE_NXDOMAIN;
                    self.note(dev, None, Note::ResolveNegative { name: cand.clone(), nx });
                    if let Some(l) = self.lookup_mut(ni, id) {
                        l.negative(nx);
                    }
                    continue;
                }
                None => {}
            }
            if l.servers.is_empty() {
                self.lookup_finish(ni, id, Err(LookupErr::NoServer));
                return;
            }
            // a fresh socket per candidate, as glibc opens one per query
            let old = l.port;
            if let Some(os) = self.os_mut(ni) {
                os.socks.retain(|s| !(s.port == old && s.owner == SockOwner::Resolver(id)));
            }
            let port = self.sock_bind(ni, 0, None, false, SockOwner::Resolver(id)).unwrap_or(0);
            let qid = match self.nodes.get_mut(ni) {
                Some(Some(n)) => n.rng.next_u32() as u16,
                _ => 0,
            };
            if let Some(l) = self.lookup_mut(ni, id) {
                l.port = port;
                l.qid = qid;
                l.server = 0;
                l.round = 0;
            }
            self.lookup_send(ni, id);
            return;
        }
    }

    /// asks the current server
    fn lookup_send(&mut self, ni: usize, id: u32) {
        let now = self.now;
        let dev = self.dev_of(ni);
        loop {
            let timeout = self.os(ni).map(|o| o.resolver.conf().timeout).unwrap_or(5 * SEC);
            let Some(l) = self.os(ni).and_then(|o| o.resolver.lookups.get(&id)) else {
                return;
            };
            let (cand, qtype, server, port, qid) = (l.cands[l.cand].clone(), l.qtype, l.servers[l.server], l.port, l.qid);
            let q = Message::query(qid, &cand, qtype, true).to_bytes();
            self.note(dev, None, Note::ResolveQuery { name: cand.clone(), qtype, server });
            let sent = self.udp_send(ni, None, port, server, PORT_DNS, &q, None);
            let Some(l) = self.lookup_mut(ni, id) else {
                return;
            };
            l.tries.push(Try { name: cand, server, sent: now, outcome: if sent { None } else { Some(TryOut::NoRoute) }, rtt: 0 });
            if sent {
                l.deadline = Some(now + timeout);
                break;
            }
            // no route to this server: the next one at once
            l.last_err = LookupErr::Timeout;
            if !self.lookup_rotate(ni, id) {
                self.lookup_finish(ni, id, Err(LookupErr::Timeout));
                return;
            }
        }
        self.arm_resolver(ni);
    }

    /// the next server (and round); false when every server was asked often enough
    fn lookup_rotate(&mut self, ni: usize, id: u32) -> bool {
        let attempts = self.os(ni).map(|o| o.resolver.conf().attempts).unwrap_or(2).max(1);
        let Some(l) = self.lookup_mut(ni, id) else {
            return false;
        };
        l.server += 1;
        if l.server >= l.servers.len() {
            l.server = 0;
            l.round += 1;
        }
        l.round < attempts
    }

    fn arm_resolver(&mut self, ni: usize) {
        let next = self.os(ni).and_then(|o| o.resolver.lookups.values().filter(|l| l.done.is_none()).filter_map(|l| l.deadline).min());
        if let Some(t) = next {
            self.set_timer(ni, SLOT_RESOLVER, t);
        }
    }

    pub(crate) fn resolver_timer(&mut self, ni: usize) {
        let now = self.now;
        let dev = self.dev_of(ni);
        let late: Vec<u32> = match self.os(ni) {
            Some(o) => o.resolver.lookups.values().filter(|l| l.done.is_none() && l.deadline.is_some_and(|d| d <= now)).map(|l| l.id).collect(),
            None => return,
        };
        for id in late {
            let Some(l) = self.lookup_mut(ni, id) else {
                continue;
            };
            l.deadline = None;
            l.last_err = LookupErr::Timeout;
            let (name, server) = (l.cands[l.cand].clone(), l.servers[l.server]);
            if let Some(t) = l.tries.last_mut() {
                t.outcome = Some(TryOut::Timeout);
            }
            self.note(dev, None, Note::ResolveTimeout { name, server });
            // nobody answers at all: the lookup ends (glibc does not search on)
            if self.lookup_rotate(ni, id) {
                self.lookup_send(ni, id);
            } else {
                self.lookup_finish(ni, id, Err(LookupErr::Timeout));
            }
        }
        self.arm_resolver(ni);
    }

    /// a datagram on the socket of a lookup
    pub(crate) fn resolver_input(&mut self, ni: usize, id: u32, d: Dgram) {
        let now = self.now;
        let dev = self.dev_of(ni);
        let Some(l) = self.os(ni).and_then(|o| o.resolver.lookups.get(&id)) else {
            return;
        };
        if l.done.is_some() || l.deadline.is_none() || d.sport != PORT_DNS || d.dport != l.port || d.src != l.servers[l.server] {
            return;
        }
        let Ok(m) = Message::parse(&d.data) else {
            return;
        };
        let cand = l.cands[l.cand].clone();
        let qtype = l.qtype;
        if !m.header.qr || m.header.id != l.qid || !m.questions.first().is_some_and(|q| q.qtype == qtype && dns::name_eq(&q.name, &cand)) {
            return;
        }
        let server = d.src;
        let rcode = m.header.rcode;
        let (canon, hits, ttl) = follow(&m.answers, &cand, qtype);
        let out = match rcode {
            dns::RCODE_NOERROR if !hits.is_empty() => TryOut::Answer,
            dns::RCODE_NOERROR => TryOut::NoData,
            dns::RCODE_NXDOMAIN => TryOut::NxDomain,
            dns::RCODE_REFUSED => TryOut::Refused,
            _ => TryOut::ServFail,
        };
        let cache_on = self.os(ni).is_some_and(|o| o.resolver.cache_on);
        let Some(l) = self.lookup_mut(ni, id) else {
            return;
        };
        l.deadline = None;
        l.reply = Some(m.clone());
        if let Some(t) = l.tries.last_mut() {
            t.outcome = Some(out);
            t.rtt = now - t.sent;
        }
        let key = (cand.to_ascii_lowercase(), qtype);
        match out {
            TryOut::Answer => {
                let addrs: Vec<Ipv4> = hits.iter().filter_map(|r| if let RData::A(a) = r.data { Some(a) } else { None }).collect();
                if cache_on {
                    self.cache_put(ni, key, CacheEntry { at: now, ttl, rcode: 0, records: m.answers.clone() });
                }
                self.lookup_finish(ni, id, Ok(Answer { name: cand, canonical: canon, addrs, records: m.answers.clone(), from: Source::Server(server), ttl }));
            }
            TryOut::NxDomain | TryOut::NoData => {
                let nx = out == TryOut::NxDomain;
                if cache_on {
                    self.cache_put(ni, key, CacheEntry { at: now, ttl: neg_ttl(&m), rcode, records: Vec::new() });
                }
                self.note(dev, Some(d.hop), Note::ResolveNegative { name: cand, nx });
                if let Some(l) = self.lookup_mut(ni, id) {
                    l.negative(nx);
                }
                self.lookup_next(ni, id);
            }
            _ => {
                // the next server; when all failed, the next candidate (glibc's got_servfail)
                if let Some(l) = self.lookup_mut(ni, id) {
                    l.last_err = if out == TryOut::Refused { LookupErr::Refused } else { LookupErr::ServFail };
                }
                if self.lookup_rotate(ni, id) {
                    self.lookup_send(ni, id);
                } else {
                    if let Some(l) = self.lookup_mut(ni, id) {
                        l.cand += 1;
                    }
                    self.lookup_next(ni, id);
                }
            }
        }
        self.arm_resolver(ni);
    }

    fn cache_put(&mut self, ni: usize, key: (String, u16), e: CacheEntry) {
        let Some(os) = self.os_mut(ni) else {
            return;
        };
        let c = &mut os.resolver.cache;
        if c.len() >= CACHE_MAX && !c.contains_key(&key) {
            if let Some(oldest) = c.iter().min_by_key(|(_, e)| e.at).map(|(k, _)| k.clone()) {
                c.remove(&oldest);
            }
        }
        if e.ttl > 0 {
            c.insert(key, e);
        }
    }

    /// a lookup is done: its socket closes, its owner hears of it
    fn lookup_finish(&mut self, ni: usize, id: u32, r: Result<Answer, LookupErr>) {
        let now = self.now;
        let dev = self.dev_of(ni);
        let Some(l) = self.lookup_mut(ni, id) else {
            return;
        };
        l.done = Some(now);
        l.deadline = None;
        let (owner, port, name, took) = (l.owner, l.port, l.name.clone(), now - l.started);
        let addr = r.as_ref().ok().and_then(|a| a.addrs.first().copied());
        let note = match &r {
            Ok(a) => Note::ResolveAnswer { name: a.name.clone(), addr, cached: a.from == Source::Cache },
            Err(e) => Note::ResolveFailed { name: name.clone(), why: *e },
        };
        l.result = Some(r);
        if let Some(os) = self.os_mut(ni) {
            os.socks.retain(|s| !(s.port == port && s.owner == SockOwner::Resolver(id)));
            os.resolver.latency.record(now, took);
        }
        self.note(dev, None, note);
        match owner {
            Owner::Cli => {}
            Owner::Svc(s) => self.os_push(ni, Job::Ev(s, SvcEvent::Resolved(id))),
            Owner::Ping(job) => self.ping_resolved(job, addr),
            Owner::Trace(job) => self.trace_resolved(job, addr),
        }
    }
}
