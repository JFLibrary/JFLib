// resolver -- name lookups of a device: the equivalent of /etc/resolv.conf with a cache, as
// systemd-resolved keeps one on Linux and IOS keeps its host cache
//
//   ResolvConf { servers, search, ndots, timeout, attempts }   what a device asks and how
//   Resolver                 the static settings (resolv.conf, "ip name-server", "ip
//                            domain-name") and the ones from DHCP; static ones win
//   Engine::lookup(dev, name, qtype) -> lookup id              starts a lookup
//   Engine::lookup_host(dev, name, Family) -> lookup id        a host's addresses as
//                            getaddrinfo finds them (below)
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
// ids and source ports come from the device's random numbers.  Name servers may be IPv4 or
// IPv6 addresses (the socket of a lookup takes both families when one of them is IPv6).
//
// lookup_host is getaddrinfo with AI_ADDRCONFIG: Family::Any asks for A and AAAA at once (one
// socket, A first, each query with its own id, as glibc sends them) when the device has an
// IPv4 address and uses IPv6 with an address other than ::1, for AAAA alone on a device with
// IPv6 only, and for A alone on a device without IPv6 (so it asks exactly as before IPv6
// came); Family::V4 / V6 ask for one type (ping -4 / -6).  Both answers of a candidate are
// awaited: one with addresses ends the lookup (the other may be empty, or time out after one
// with addresses came), two negative ones go on to the next candidate.  Answer::sorted lists
// the addresses in the order getaddrinfo returns them: RFC 6724 destination address selection
// (netpkt::addrsel: unusable ones last, matching scope, no deprecated source, matching label,
// higher precedence, smaller scope, the longest matching prefix between IPv6 destinations, else
// the order of the answers), each with the source address the device would use; so a host with
// a global IPv6 address tries IPv6 first, one with only a link-local IPv6 address IPv4 first.
// An A lookup alone keeps the order of its answer.

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

use base::metrics::Window;

use netpkt::addrsel::{self, Dest};
use netpkt::dns::{self, Message, RData, Rr, CLASS_IN, TYPE_A, TYPE_AAAA, TYPE_CNAME, TYPE_SOA};
use netpkt::udp::PORT_DNS;
use netpkt::{IpAddr, Ipv4, Ipv6};

use crate::action::ActErr;
use crate::kernel::{Engine, Ns, SEC};
use crate::os::{Dgram, Job, Owner as SockOwner, SvcEvent, ANY, ANY4, SLOT_RESOLVER};
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
    pub servers: Vec<IpAddr>,
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

/// the address families a host lookup asks for (getaddrinfo's ai_family)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Family {
    /// what the device can use (AF_UNSPEC with AI_ADDRCONFIG)
    Any,
    /// IPv4 only (AF_INET: "-4")
    V4,
    /// IPv6 only (AF_INET6: "-6")
    V6,
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
    /// an address of the other family where one family was asked for (EAI_ADDRFAMILY)
    Family,
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
            LookupErr::Family => "eine Adresse der anderen Familie",
        }
    }
}

/// where an answer came from
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Literal,
    Hosts,
    Cache,
    Server(IpAddr),
}

/// a successful lookup
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Answer {
    /// the candidate that was answered ("www.lab" for "www")
    pub name: String,
    /// the end of the CNAME chain
    pub canonical: String,
    /// the IPv4 addresses (A records) in the order of the answer
    pub addrs: Vec<Ipv4>,
    /// the answer section (both answers of a lookup for A and AAAA, A first)
    pub records: Vec<Rr>,
    pub from: Source,
    /// seconds it stays valid
    pub ttl: u32,
    /// the IPv6 addresses (AAAA records) in the order of the answer
    pub addrs6: Vec<Ipv6>,
    /// both kinds in the order to try them (RFC 6724; an A lookup alone: the answer's order)
    pub sorted: Vec<IpAddr>,
}

impl Answer {
    /// the address to use first
    pub fn first(&self) -> Option<IpAddr> {
        self.sorted.first().copied()
    }
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
    pub server: IpAddr,
    pub sent: Ns,
    /// None while waiting
    pub outcome: Option<TryOut>,
    pub rtt: Ns,
    /// the type asked for
    pub qtype: u16,
}

/// an answer of the current round: its type, the outcome, the answer section, the TTL to keep
/// it (seconds left for one from the cache) and the CNAME chain's end
#[derive(Clone, Debug, PartialEq, Eq)]
struct Got {
    qtype: u16,
    out: TryOut,
    records: Vec<Rr>,
    ttl: u32,
    canon: String,
    cached: bool,
}

/// a lookup in progress or done
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Lookup {
    pub id: u32,
    pub name: String,
    pub qtype: u16,
    /// asked together with qtype: AAAA beside A (a host lookup for both families)
    pub qtype2: Option<u16>,
    pub owner: Owner,
    pub started: Ns,
    pub done: Option<Ns>,
    pub result: Option<Result<Answer, LookupErr>>,
    pub tries: Vec<Try>,
    /// the last DNS reply (dig shows it)
    pub reply: Option<Message>,
    cands: Vec<String>,
    cand: usize,
    servers: Vec<IpAddr>,
    server: usize,
    round: u8,
    qid: u16,
    qid2: u16,
    port: u16,
    deadline: Option<Ns>,
    last_err: LookupErr,
    /// the error of the name as it is, when it was asked first (dots >= ndots)
    asis_err: Option<LookupErr>,
    nodata: bool,
    /// the answers of the current candidate so far
    got: Vec<Got>,
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
    /// the types it asks for, in the order they go out
    fn types(&self) -> Vec<u16> {
        let mut v = alloc::vec![self.qtype];
        v.extend(self.qtype2);
        v
    }
    /// the types the current round still waits for
    fn missing(&self) -> Vec<u16> {
        self.types().into_iter().filter(|t| !self.got.iter().any(|g| g.qtype == *t)).collect()
    }
    /// the query id of a type
    fn qid_of(&self, t: u16) -> u16 {
        if Some(t) == self.qtype2 && t != self.qtype { self.qid2 } else { self.qid }
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
                c.servers = s.iter().map(|a| IpAddr::V4(*a)).collect();
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
pub(crate) fn candidates(name: &str, c: &ResolvConf) -> Vec<String> {
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

/// the addresses of records (A and AAAA at the end of their chains)
fn addrs_of(records: &[Rr], name: &str) -> (String, Vec<Ipv4>, Vec<Ipv6>) {
    let (canon4, a, _) = follow(records, name, TYPE_A);
    let (canon6, aaaa, _) = follow(records, name, TYPE_AAAA);
    let v4: Vec<Ipv4> = a.iter().filter_map(|r| if let RData::A(x) = r.data { Some(x) } else { None }).collect();
    let v6: Vec<Ipv6> = aaaa.iter().filter_map(|r| if let RData::Aaaa(x) = r.data { Some(x) } else { None }).collect();
    (if v4.is_empty() && !v6.is_empty() { canon6 } else { canon4 }, v4, v6)
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

    /// starts a lookup of a host's addresses as getaddrinfo does (see the header)
    pub fn lookup_host_of(&mut self, dev: u32, name: &str, fam: Family) -> Result<u32, ActErr> {
        let ni = self.node_index(dev).ok_or(ActErr::UnknownDevice(dev))?;
        if self.os(ni).is_none() {
            return Err(ActErr::NoIp(dev));
        }
        Ok(self.lookup_host(ni, name, fam, Owner::Cli))
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

    /// the families a device can use for a host lookup (AI_ADDRCONFIG): an IPv4 address other
    /// than 127/8, and IPv6 with an address other than ::1
    pub(crate) fn lookup_families(&self, ni: usize) -> (bool, bool) {
        let v4 = self.stack(ni).is_some_and(|s| s.ifaces.iter().any(|f| !f.hidden && f.addrs.iter().any(|a| !a.addr.is_loopback())));
        let v6 = self.v6(ni).is_some_and(|v| v.used() && v.ifs.iter().any(|f| f.up && f.addrs.iter().any(|a| a.state.usable() && !a.addr.is_loopback())));
        (v4, v6)
    }

    /// a host lookup of node `ni` (getaddrinfo, see the header)
    pub(crate) fn lookup_host(&mut self, ni: usize, name: &str, fam: Family, owner: Owner) -> u32 {
        let (qtype, qtype2) = match fam {
            Family::V4 => (TYPE_A, None),
            Family::V6 => (TYPE_AAAA, None),
            Family::Any => match self.lookup_families(ni) {
                (_, false) => (TYPE_A, None),
                (false, true) => (TYPE_AAAA, None),
                (true, true) => (TYPE_A, Some(TYPE_AAAA)),
            },
        };
        self.lookup_begin(ni, name, qtype, qtype2, owner)
    }

    pub(crate) fn lookup_start(&mut self, ni: usize, name: &str, qtype: u16, owner: Owner) -> u32 {
        self.lookup_begin(ni, name, qtype, None, owner)
    }

    fn lookup_begin(&mut self, ni: usize, name: &str, qtype: u16, qtype2: Option<u16>, owner: Owner) -> u32 {
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
            qtype2,
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
            qid2: 0,
            port: 0,
            deadline: None,
            last_err: LookupErr::NotFound,
            asis_err: None,
            nodata: false,
            got: Vec::new(),
        };
        let wants4 = qtype == TYPE_A;
        let wants6 = qtype == TYPE_AAAA || qtype2 == Some(TYPE_AAAA);
        // a literal, the host table
        let parsed = if wants4 || wants6 { IpAddr::parse(&name) } else { None };
        let literal = match parsed {
            Some(IpAddr::V4(a)) if wants4 => Some(IpAddr::V4(a)),
            Some(IpAddr::V6(a)) if wants6 => Some(IpAddr::V6(a)),
            _ => None,
        };
        // an address of the other family is no name to ask for (getaddrinfo: EAI_ADDRFAMILY)
        let other = parsed.is_some() && literal.is_none();
        let bare = String::from(name.trim_end_matches('.'));
        let mut hosted: Vec<IpAddr> = Vec::new();
        if wants4 || wants6 {
            if bare.eq_ignore_ascii_case("localhost") {
                if wants4 {
                    hosted.push(IpAddr::V4(Ipv4::LOCALHOST));
                }
                if wants6 {
                    hosted.push(IpAddr::V6(Ipv6::LOCALHOST));
                }
            } else {
                hosted.extend(os.hosts.iter().filter(|(h, a)| h.eq_ignore_ascii_case(&bare) && (if a.is_v4() { wants4 } else { wants6 })).map(|h| h.1));
            }
        }
        os.resolver.lookups.insert(id, l);
        if other {
            self.lookup_finish(ni, id, Err(LookupErr::Family));
            return id;
        }
        if let Some(ip) = literal {
            let (v4, v6) = split(&[ip]);
            self.lookup_finish(ni, id, Ok(Answer { name: name.clone(), canonical: name, addrs: v4, records: Vec::new(), from: Source::Literal, ttl: 0, addrs6: v6, sorted: Vec::new() }));
            return id;
        }
        if !hosted.is_empty() {
            let (v4, v6) = split(&hosted);
            self.lookup_finish(ni, id, Ok(Answer { name: bare.clone(), canonical: bare, addrs: v4, records: Vec::new(), from: Source::Hosts, ttl: 0, addrs6: v6, sorted: Vec::new() }));
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
            // what the cache knows of each type
            let mut got: Vec<Got> = Vec::new();
            if os.resolver.cache_on {
                for t in l.types() {
                    if let Some((e, left)) = os.resolver.cached(now, &cand, t) {
                        let (canon, hits, _) = follow(&e.records, &cand, t);
                        let out = if e.rcode == 0 && !e.records.is_empty() && !hits.is_empty() {
                            TryOut::Answer
                        } else if e.rcode == 0 && !e.records.is_empty() {
                            TryOut::NoData
                        } else if e.rcode == dns::RCODE_NXDOMAIN {
                            TryOut::NxDomain
                        } else {
                            TryOut::NoData
                        };
                        // a positive entry of the old kind (records without a hit) still answers
                        let out = if e.rcode == 0 && !e.records.is_empty() && l.qtype2.is_none() { TryOut::Answer } else { out };
                        got.push(Got { qtype: t, out, records: e.records.clone(), ttl: left, canon, cached: true });
                    }
                }
            }
            let all = got.len() == l.types().len();
            if let Some(l) = self.lookup_mut(ni, id) {
                l.got = got;
            }
            if all {
                match self.round_result(ni, id) {
                    Some(Ok(a)) => {
                        self.lookup_finish(ni, id, Ok(a));
                        return;
                    }
                    Some(Err(nx)) => {
                        self.note(dev, None, Note::ResolveNegative { name: cand.clone(), nx });
                        if let Some(l) = self.lookup_mut(ni, id) {
                            l.got.clear();
                            l.negative(nx);
                        }
                        continue;
                    }
                    None => {}
                }
            }
            let Some(l) = self.os(ni).and_then(|o| o.resolver.lookups.get(&id)) else {
                return;
            };
            if l.servers.is_empty() {
                self.lookup_finish(ni, id, Err(LookupErr::NoServer));
                return;
            }
            // a fresh socket per candidate, as glibc opens one per query (both families when a
            // server is an IPv6 address)
            let old = l.port;
            let bind = if l.servers.iter().any(|s| !s.is_v4()) { ANY } else { ANY4 };
            let two = l.qtype2.is_some() && l.missing().len() > 1;
            if let Some(os) = self.os_mut(ni) {
                os.socks.retain(|s| !(s.port == old && s.owner == SockOwner::Resolver(id)));
            }
            let port = self.sock_bind(ni, bind, 0, None, false, SockOwner::Resolver(id)).unwrap_or(0);
            let mut draw = || match self.nodes.get_mut(ni) {
                Some(Some(n)) => n.rng.next_u32() as u16,
                _ => 0,
            };
            let qid = draw();
            let qid2 = if two { draw() } else { qid };
            if let Some(l) = self.lookup_mut(ni, id) {
                l.port = port;
                l.qid = qid;
                l.qid2 = qid2;
                l.server = 0;
                l.round = 0;
            }
            self.lookup_send(ni, id);
            return;
        }
    }

    /// what the answers of the current round say once every type is in: Ok with the
    /// addresses, Err(nx) when all are negative (nx: every one NXDOMAIN), None when a server
    /// failed (SERVFAIL, REFUSED) and nothing else answered
    fn round_result(&self, ni: usize, id: u32) -> Option<Result<Answer, bool>> {
        let l = self.os(ni)?.resolver.lookups.get(&id)?;
        let cand = l.cands.get(l.cand)?.clone();
        let pos: Vec<&Got> = l.got.iter().filter(|g| g.out == TryOut::Answer).collect();
        if !pos.is_empty() {
            let mut records: Vec<Rr> = Vec::new();
            for t in l.types() {
                if let Some(g) = l.got.iter().find(|g| g.qtype == t) {
                    records.extend(g.records.iter().cloned());
                }
            }
            let (_, v4, v6) = addrs_of(&records, &cand);
            let canon = pos[0].canon.clone();
            let ttl = pos.iter().map(|g| g.ttl).min().unwrap_or(0);
            let from = if pos.iter().all(|g| g.cached) { Source::Cache } else { Source::Server(l.servers.get(l.server).copied().unwrap_or(ANY4)) };
            // one type alone keeps its answer's addresses (an A lookup has no AAAA ones)
            let v4 = if l.types().contains(&TYPE_A) { v4 } else { Vec::new() };
            let v6 = if l.types().contains(&TYPE_AAAA) { v6 } else { Vec::new() };
            return Some(Ok(Answer { name: cand, canonical: canon, addrs: v4, records, from, ttl, addrs6: v6, sorted: Vec::new() }));
        }
        if l.got.iter().any(|g| matches!(g.out, TryOut::ServFail | TryOut::Refused)) {
            return None;
        }
        Some(Err(l.got.iter().all(|g| g.out == TryOut::NxDomain)))
    }

    /// asks the current server what the round still lacks (A before AAAA)
    fn lookup_send(&mut self, ni: usize, id: u32) {
        let now = self.now;
        let dev = self.dev_of(ni);
        loop {
            let timeout = self.os(ni).map(|o| o.resolver.conf().timeout).unwrap_or(5 * SEC);
            let Some(l) = self.os(ni).and_then(|o| o.resolver.lookups.get(&id)) else {
                return;
            };
            let (cand, server, port) = (l.cands[l.cand].clone(), l.servers[l.server], l.port);
            let asks: Vec<(u16, u16)> = l.missing().into_iter().map(|t| (t, l.qid_of(t))).collect();
            let mut any = false;
            for (qtype, qid) in asks {
                let q = Message::query(qid, &cand, qtype, true).to_bytes();
                self.note(dev, None, Note::ResolveQuery { name: cand.clone(), qtype, server });
                let sent = self.udp_send(ni, None, port, server, PORT_DNS, &q, None);
                any |= sent;
                let Some(l) = self.lookup_mut(ni, id) else {
                    return;
                };
                l.tries.push(Try { name: cand.clone(), server, sent: now, outcome: if sent { None } else { Some(TryOut::NoRoute) }, rtt: 0, qtype });
            }
            let Some(l) = self.lookup_mut(ni, id) else {
                return;
            };
            if any {
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
            for t in l.tries.iter_mut().filter(|t| t.outcome.is_none()) {
                t.outcome = Some(TryOut::Timeout);
            }
            // one type answered with addresses, the other did not come: what came is the answer
            if l.got.iter().any(|g| g.out == TryOut::Answer)
                && let Some(Ok(a)) = self.round_result(ni, id)
            {
                self.lookup_finish(ni, id, Ok(a));
                continue;
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
        let missing = l.missing();
        let Some(qtype) = missing.iter().copied().find(|&t| m.header.id == l.qid_of(t) && m.questions.first().is_some_and(|q| q.qtype == t && dns::name_eq(&q.name, &cand))) else {
            return;
        };
        if !m.header.qr {
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
        let last = missing.len() == 1;
        if last {
            l.deadline = None;
        }
        l.reply = Some(m.clone());
        if let Some(t) = l.tries.iter_mut().rev().find(|t| t.qtype == qtype && t.outcome.is_none()) {
            t.outcome = Some(out);
            t.rtt = now - t.sent;
        }
        let key = (cand.to_ascii_lowercase(), qtype);
        let gttl = if out == TryOut::Answer { ttl } else { neg_ttl(&m) };
        l.got.push(Got { qtype, out, records: if out == TryOut::Answer { m.answers.clone() } else { Vec::new() }, ttl: gttl, canon, cached: false });
        match out {
            TryOut::Answer => {
                if cache_on {
                    self.cache_put(ni, key, CacheEntry { at: now, ttl, rcode: 0, records: m.answers.clone() });
                }
            }
            TryOut::NxDomain | TryOut::NoData => {
                let nx = out == TryOut::NxDomain;
                if cache_on {
                    self.cache_put(ni, key, CacheEntry { at: now, ttl: neg_ttl(&m), rcode, records: Vec::new() });
                }
                self.note(dev, Some(d.hop), Note::ResolveNegative { name: cand.clone(), nx });
            }
            _ => {}
        }
        if !last {
            // the other answer of the round is still to come
            self.arm_resolver(ni);
            return;
        }
        match self.round_result(ni, id) {
            Some(Ok(a)) => {
                let a = Answer { from: Source::Server(server), ..a };
                self.lookup_finish(ni, id, Ok(a));
            }
            Some(Err(nx)) => {
                if let Some(l) = self.lookup_mut(ni, id) {
                    l.got.clear();
                    l.negative(nx);
                }
                self.lookup_next(ni, id);
            }
            None => {
                // the next server; when all failed, the next candidate (glibc's got_servfail)
                if let Some(l) = self.lookup_mut(ni, id) {
                    l.last_err = if l.got.iter().any(|g| g.out == TryOut::Refused) { LookupErr::Refused } else { LookupErr::ServFail };
                    l.got.retain(|g| !matches!(g.out, TryOut::ServFail | TryOut::Refused));
                }
                if self.lookup_rotate(ni, id) {
                    self.lookup_send(ni, id);
                } else {
                    if let Some(l) = self.lookup_mut(ni, id) {
                        l.got.clear();
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

    /// the addresses of an answer in the order to try them: an A lookup alone in the order of
    /// the answer, else RFC 6724 with the source address each would leave with
    fn lookup_sorted(&self, ni: usize, v4: &[Ipv4], v6: &[Ipv6]) -> Vec<IpAddr> {
        let all: Vec<IpAddr> = v4.iter().map(|a| IpAddr::V4(*a)).chain(v6.iter().map(|a| IpAddr::V6(*a))).collect();
        if v6.is_empty() {
            return all;
        }
        let mut d: Vec<Dest> = all
            .iter()
            .map(|&a| {
                let src = self.source_ip(ni, a);
                let mut x = Dest::new(a, src);
                if let (Some(IpAddr::V6(s)), Some(v)) = (src, self.v6(ni))
                    && let Some(e) = v.ifs.iter().find_map(|f| f.addr(s))
                {
                    x.src_len = e.len;
                    x.src_deprecated = e.state == crate::ip6::AddrState::Deprecated;
                }
                x
            })
            .collect();
        addrsel::sort(&mut d);
        d.into_iter().map(|x| x.addr).collect()
    }

    /// a lookup is done: its socket closes, its owner hears of it
    fn lookup_finish(&mut self, ni: usize, id: u32, r: Result<Answer, LookupErr>) {
        let now = self.now;
        let dev = self.dev_of(ni);
        let r = r.map(|a| {
            let sorted = self.lookup_sorted(ni, &a.addrs, &a.addrs6);
            Answer { sorted, ..a }
        });
        let Some(l) = self.lookup_mut(ni, id) else {
            return;
        };
        l.done = Some(now);
        l.deadline = None;
        let (owner, port, name, took) = (l.owner, l.port, l.name.clone(), now - l.started);
        let addr = r.as_ref().ok().and_then(|a| a.first());
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

/// addresses by family
fn split(a: &[IpAddr]) -> (Vec<Ipv4>, Vec<Ipv6>) {
    (a.iter().filter_map(|x| x.v4()).collect(), a.iter().filter_map(|x| x.v6()).collect())
}
