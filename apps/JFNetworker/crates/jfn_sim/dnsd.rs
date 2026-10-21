// dnsd -- the authoritative DNS server of a device (bind9 on Linux, "ip dns server" on IOS):
// zones with SOA, NS, A, AAAA, CNAME, PTR, MX and TXT records; other names can go to the
// device's own resolver (a caching forwarder, as dnsmasq or IOS with "ip domain-lookup")
//
//   DnsServer::new(ios)                kind "dns-server"
//   Zone { origin, ttl, records }      Zone::new(origin): an SOA (ns.<origin>,
//                                      hostmaster.<origin>) and an NS record; add / remove
//   parse_record(line, origin, ttl)    a record in zone-file form: "www 300 CNAME web",
//                                      "@ MX 10 mail", "web A 10.0.2.20" (relative names get the
//                                      origin, names ending in a dot are absolute)
//   Engine::dns_zones(dev)
//
// Answers (RFC 1034 4.3.2): the zone with the longest origin that holds the name; below a
// delegation (NS records under the origin) a referral with glue; else the records of the
// type (or ANY); else a CNAME, followed inside the zone for up to 8 steps; else NODATA when
// the name exists (as an empty non-terminal too) or NXDOMAIN, both with the SOA in the
// authority section, its TTL the smaller of SOA TTL and minimum (RFC 2308).  Names outside
// every zone: REFUSED, or, with forwarding on and recursion desired, the device's resolver
// looks them up (RA set).  Over UDP an answer is cut at 512 bytes (TC) unless the query offers
// more with EDNS: then at most 1232 bytes, and an OPT record goes back.  The answer comes
// from the address the query was sent to.

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::any::Any;

use base::metrics::Counter;
use netpkt::dns::{self, Message, RData, Rr, Soa, CLASS_IN, TYPE_ANY, TYPE_CNAME, TYPE_NS, TYPE_OPT, TYPE_SOA};
use netpkt::udp::PORT_DNS;
use netpkt::{Ipv4, Ipv6};

use crate::kernel::Engine;
use crate::os::{Dgram, Metric, OsApi, Service, SvcEvent};
use crate::resolver::LookupErr;
use crate::trace::Note;

/// the largest UDP answer with EDNS (the DNS flag day 2020 value)
pub const EDNS_MAX: usize = 1232;

/// a zone the server is authoritative for
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Zone {
    pub origin: String,
    /// the TTL of records added without one
    pub ttl: u32,
    pub records: Vec<Rr>,
}

impl Zone {
    /// a zone with an SOA and an NS record (serial 1, refresh 3600, retry 600, expire 86400,
    /// minimum 300)
    pub fn new(origin: &str) -> Zone {
        let o = String::from(origin.trim_end_matches('.'));
        let sub = |h: &str| if o.is_empty() { String::from(h) } else { format!("{}.{}", h, o) };
        let soa = Soa { mname: sub("ns"), rname: sub("hostmaster"), serial: 1, refresh: 3600, retry: 600, expire: 86_400, minimum: 300 };
        let records = alloc::vec![Rr::new(&o, 3600, RData::Soa(soa)), Rr::new(&o, 3600, RData::Ns(sub("ns")))];
        Zone { origin: o, ttl: 3600, records }
    }
    pub fn soa(&self) -> Option<(&Rr, &Soa)> {
        self.records.iter().find_map(|r| match &r.data {
            RData::Soa(s) if r.rtype == TYPE_SOA => Some((r, s)),
            _ => None,
        })
    }
    /// adds a record (an SOA replaces the SOA, an equal record is not added twice)
    pub fn add(&mut self, rr: Rr) -> bool {
        if !dns::is_subdomain(&rr.name, &self.origin) {
            return false;
        }
        if rr.rtype == TYPE_SOA {
            self.records.retain(|r| r.rtype != TYPE_SOA);
            self.records.insert(0, rr);
            return true;
        }
        if self.records.iter().any(|r| r.rtype == rr.rtype && dns::name_eq(&r.name, &rr.name) && r.data == rr.data) {
            return false;
        }
        self.records.push(rr);
        true
    }
    /// removes the records with the name, type and data (data Other(empty): every record of the
    /// name and type)
    pub fn remove(&mut self, rr: &Rr) -> bool {
        let n = self.records.len();
        let any = rr.data == RData::Other(Vec::new());
        self.records.retain(|r| !(r.rtype == rr.rtype && dns::name_eq(&r.name, &rr.name) && (any || r.data == rr.data)));
        self.records.len() != n
    }
    fn at<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Rr> + 'a {
        self.records.iter().filter(move |r| dns::name_eq(&r.name, name))
    }
    /// the name has records or names below it
    fn exists(&self, name: &str) -> bool {
        self.records.iter().any(|r| dns::is_subdomain(&r.name, name))
    }
    /// the TTL of negative answers (RFC 2308 5)
    fn neg_soa(&self) -> Option<Rr> {
        let (r, s) = self.soa()?;
        let mut rr = r.clone();
        rr.ttl = r.ttl.min(s.minimum);
        Some(rr)
    }
}

/// a name in a zone file: "@" is the origin, a final dot makes it absolute, else the origin is
/// appended
fn zone_name(t: &str, origin: &str) -> String {
    if t == "@" {
        return String::from(origin);
    }
    if let Some(abs) = t.strip_suffix('.') {
        return String::from(abs);
    }
    if origin.is_empty() { String::from(t) } else { format!("{}.{}", t, origin) }
}

/// the next word of `r` (r moves past it)
fn word(r: &mut &str) -> Option<String> {
    let w: String = r.split_whitespace().next()?.into();
    *r = r.trim_start()[w.len()..].trim_start();
    Some(w)
}

/// a record in zone-file form: NAME [TTL] [IN] TYPE DATA
pub fn parse_record(line: &str, origin: &str, ttl: u32) -> Option<Rr> {
    let line = line.trim();
    let (name, rest) = line.split_once(char::is_whitespace)?;
    let mut rest = rest.trim_start();
    let mut ttl = ttl;
    let mut t = word(&mut rest)?;
    if t.bytes().all(|b| b.is_ascii_digit()) {
        ttl = t.parse().ok()?;
        t = word(&mut rest)?;
    }
    if t.eq_ignore_ascii_case("IN") {
        t = word(&mut rest)?;
    }
    let owner = zone_name(name, origin);
    let n = |s: &str| zone_name(s, origin);
    let data = match t.to_ascii_uppercase().as_str() {
        "A" => RData::A(Ipv4::parse(rest.trim())?),
        "AAAA" => RData::Aaaa(Ipv6::parse(rest.trim())?),
        "CNAME" => RData::Cname(n(rest.trim())),
        "NS" => RData::Ns(n(rest.trim())),
        "PTR" => RData::Ptr(n(rest.trim())),
        "MX" => {
            let pref: u16 = word(&mut rest)?.parse().ok()?;
            RData::Mx { pref, host: n(rest.trim()) }
        }
        "TXT" => {
            let mut v = Vec::new();
            let s = rest.trim();
            if s.starts_with('"') {
                let (mut cur, mut inside, mut esc) = (Vec::new(), false, false);
                for &c in s.as_bytes() {
                    if !inside {
                        inside = c == b'"';
                    } else if esc {
                        cur.push(c);
                        esc = false;
                    } else if c == b'\\' {
                        esc = true;
                    } else if c == b'"' {
                        v.push(core::mem::take(&mut cur));
                        inside = false;
                    } else {
                        cur.push(c);
                    }
                }
            } else {
                v.extend(s.split_whitespace().map(|w| w.as_bytes().to_vec()));
            }
            RData::Txt(v)
        }
        "SOA" => {
            let w: Vec<&str> = rest.split_whitespace().collect();
            if w.len() != 7 {
                return None;
            }
            let num = |i: usize| w[i].parse::<u32>().ok();
            RData::Soa(Soa { mname: n(w[0]), rname: n(w[1]), serial: num(2)?, refresh: num(3)?, retry: num(4)?, expire: num(5)?, minimum: num(6)? })
        }
        _ => return None,
    };
    dns::labels(&owner)?;
    Some(Rr::new(&owner, ttl, data))
}

/// what became of a question
enum Outcome {
    Answered(u16),
    NxDomain,
    NoData,
    Referral(String),
    Refused,
    Forward,
}

/// a question passed to the resolver
#[derive(Clone, Debug)]
struct Fwd {
    lookup: u32,
    query: Message,
    to: (Ipv4, u16),
    from: Option<Ipv4>,
    edns: Option<u16>,
    hop: u64,
}

/// the DNS server of a device
#[derive(Clone, Debug)]
pub struct DnsServer {
    pub ios: bool,
    pub zones: Vec<Zone>,
    /// names outside the zones go to the device's resolver
    pub forward: bool,
    pending: Vec<Fwd>,
    queries: Counter,
    answers: Counter,
    nxdomain: Counter,
    nodata: Counter,
    referrals: Counter,
    refused: Counter,
    forwarded: Counter,
    servfail: Counter,
}

impl DnsServer {
    pub fn new(ios: bool) -> DnsServer {
        DnsServer {
            ios,
            zones: Vec::new(),
            forward: false,
            pending: Vec::new(),
            queries: Counter::new(),
            answers: Counter::new(),
            nxdomain: Counter::new(),
            nodata: Counter::new(),
            referrals: Counter::new(),
            refused: Counter::new(),
            forwarded: Counter::new(),
            servfail: Counter::new(),
        }
    }
    pub fn zone(&self, origin: &str) -> Option<&Zone> {
        self.zones.iter().find(|z| dns::name_eq(&z.origin, origin))
    }
    pub fn zone_mut(&mut self, origin: &str) -> Option<&mut Zone> {
        self.zones.iter_mut().find(|z| dns::name_eq(&z.origin, origin))
    }
    /// the zone with the longest origin that holds `name`
    pub fn zone_for(&self, name: &str) -> Option<&Zone> {
        self.zones.iter().filter(|z| dns::is_subdomain(name, &z.origin)).max_by_key(|z| z.origin.len())
    }
    fn zone_for_mut(&mut self, name: &str) -> Option<&mut Zone> {
        self.zones.iter_mut().filter(|z| dns::is_subdomain(name, &z.origin)).max_by_key(|z| z.origin.len())
    }
    /// adds a record to the zone that holds its name; false when no zone does
    pub fn add_record(&mut self, rr: Rr) -> bool {
        match self.zone_for_mut(&rr.name) {
            Some(z) => z.add(rr),
            None => false,
        }
    }
    pub fn remove_record(&mut self, rr: &Rr) -> bool {
        self.zone_for_mut(&rr.name).is_some_and(|z| z.remove(rr))
    }

    /// fills the answer for one question
    fn answer(&self, name: &str, qtype: u16, r: &mut Message) -> Outcome {
        let Some(z) = self.zone_for(name) else {
            return if self.forward && r.header.rd { Outcome::Forward } else { Outcome::Refused };
        };
        // a delegation between the origin and the name
        let cut = z
            .records
            .iter()
            .filter(|x| x.rtype == TYPE_NS && !dns::name_eq(&x.name, &z.origin) && dns::is_subdomain(name, &x.name))
            .map(|x| x.name.clone())
            .max_by_key(|n| n.len());
        if let Some(cut) = cut {
            let ns: Vec<Rr> = z.at(&cut).filter(|x| x.rtype == TYPE_NS).cloned().collect();
            for x in &ns {
                if let RData::Ns(host) = &x.data {
                    r.additional.extend(z.at(host).filter(|g| g.rtype == dns::TYPE_A || g.rtype == dns::TYPE_AAAA).cloned());
                }
            }
            r.authority = ns;
            return Outcome::Referral(cut);
        }
        r.header.aa = true;
        let mut cur = String::from(name);
        for _ in 0..8 {
            let hits: Vec<Rr> = z.at(&cur).filter(|x| x.rtype == qtype || (qtype == TYPE_ANY && x.rtype != TYPE_OPT)).cloned().collect();
            if !hits.is_empty() {
                r.answers.extend(hits);
                return Outcome::Answered(r.answers.len() as u16);
            }
            let cname = z.at(&cur).find(|x| x.rtype == TYPE_CNAME).cloned();
            match cname {
                Some(c) if qtype != TYPE_CNAME => {
                    let RData::Cname(target) = &c.data else {
                        break;
                    };
                    let target = target.clone();
                    r.answers.push(c);
                    if !dns::is_subdomain(&target, &z.origin) {
                        // the client follows the chain outside the zone itself
                        return Outcome::Answered(r.answers.len() as u16);
                    }
                    cur = target;
                }
                _ => break,
            }
        }
        if let Some(soa) = z.neg_soa() {
            r.authority.push(soa);
        }
        if z.exists(&cur) {
            if r.answers.is_empty() { Outcome::NoData } else { Outcome::Answered(r.answers.len() as u16) }
        } else {
            r.header.rcode = dns::RCODE_NXDOMAIN;
            Outcome::NxDomain
        }
    }

    /// sends an answer from the address the query went to
    fn send(os: &mut OsApi, r: &Message, edns: Option<u16>, to: (Ipv4, u16), from: Option<Ipv4>) {
        let mut r = r.clone();
        let limit = match edns {
            Some(sz) if sz as usize > dns::UDP_MAX => {
                r.additional.push(Rr { name: String::new(), rtype: TYPE_OPT, class: EDNS_MAX as u16, ttl: 0, data: RData::Other(Vec::new()) });
                (sz as usize).min(EDNS_MAX)
            }
            Some(_) => {
                r.additional.push(Rr { name: String::new(), rtype: TYPE_OPT, class: EDNS_MAX as u16, ttl: 0, data: RData::Other(Vec::new()) });
                dns::UDP_MAX
            }
            None => dns::UDP_MAX,
        };
        let (b, _) = r.to_bytes_max(limit);
        match from {
            Some(src) => os.send_from(src, PORT_DNS, to.0, to.1, &b),
            None => os.send(PORT_DNS, to.0, to.1, &b),
        };
    }

    fn on_query(&mut self, os: &mut OsApi, d: &Dgram) {
        let Ok(q) = Message::parse(&d.data) else {
            return;
        };
        if q.header.qr {
            return;
        }
        self.queries.inc();
        let from = if os.is_mine(d.dst) { Some(d.dst) } else { None };
        let edns = q.additional.iter().find(|r| r.rtype == TYPE_OPT).map(|r| r.class);
        let mut r = Message::reply_to(&q);
        r.header.ra = self.forward;
        let to = (d.src, d.sport);
        let Some(qq) = q.questions.first().cloned() else {
            r.header.rcode = dns::RCODE_FORMERR;
            DnsServer::send(os, &r, edns, to, from);
            return;
        };
        if q.header.opcode != dns::OPCODE_QUERY || qq.qclass != CLASS_IN {
            r.header.rcode = dns::RCODE_NOTIMP;
            DnsServer::send(os, &r, edns, to, from);
            return;
        }
        match self.answer(&qq.name, qq.qtype, &mut r) {
            Outcome::Answered(n) => {
                self.answers.inc();
                os.note(Note::DnsAnswer { name: qq.name.clone(), qtype: qq.qtype, n });
            }
            Outcome::NxDomain => {
                self.nxdomain.inc();
                os.note(Note::DnsNxDomain { name: qq.name.clone() });
            }
            Outcome::NoData => {
                self.nodata.inc();
                os.note(Note::DnsNoData { name: qq.name.clone(), qtype: qq.qtype });
            }
            Outcome::Referral(zone) => {
                self.referrals.inc();
                os.note(Note::DnsReferral { name: qq.name.clone(), zone });
            }
            Outcome::Refused => {
                self.refused.inc();
                r.header.rcode = dns::RCODE_REFUSED;
                os.note(Note::DnsRefused { name: qq.name.clone() });
            }
            Outcome::Forward => {
                self.forwarded.inc();
                os.note(Note::DnsForward { name: qq.name.clone() });
                let abs = format!("{}.", qq.name.trim_end_matches('.'));
                let lookup = os.resolve(&abs, qq.qtype);
                let hop = os.cause.unwrap_or(0);
                self.pending.push(Fwd { lookup, query: q, to, from, edns, hop });
                return;
            }
        }
        DnsServer::send(os, &r, edns, to, from);
    }

    fn on_resolved(&mut self, os: &mut OsApi, id: u32) {
        let Some(i) = self.pending.iter().position(|f| f.lookup == id) else {
            return;
        };
        let f = self.pending.remove(i);
        let mut r = Message::reply_to(&f.query);
        r.header.ra = true;
        match os.lookup(id).and_then(|l| l.result.clone()) {
            Some(Ok(a)) => r.answers = a.records,
            Some(Err(LookupErr::NotFound)) => r.header.rcode = dns::RCODE_NXDOMAIN,
            Some(Err(LookupErr::NoData)) => {}
            _ => {
                self.servfail.inc();
                r.header.rcode = dns::RCODE_SERVFAIL;
            }
        }
        os.cause = if f.hop == 0 { None } else { Some(f.hop) };
        DnsServer::send(os, &r, f.edns, f.to, f.from);
        os.cause = None;
    }
}

impl Service for DnsServer {
    fn kind(&self) -> &'static str {
        "dns-server"
    }
    fn start(&mut self, os: &mut OsApi) {
        self.pending.clear();
        if os.bind(PORT_DNS, None).is_err() {
            os.note(Note::PortInUse { port: PORT_DNS, svc: String::from("dns-server") });
        }
    }
    fn on(&mut self, ev: SvcEvent, os: &mut OsApi) {
        match ev {
            SvcEvent::Udp(d) => self.on_query(os, &d),
            SvcEvent::Resolved(id) => self.on_resolved(os, id),
            _ => {}
        }
    }
    fn metrics(&self) -> Vec<Metric> {
        alloc::vec![
            Metric::new("queries", self.queries.get() as i64),
            Metric::new("answers", self.answers.get() as i64),
            Metric::new("nxdomain", self.nxdomain.get() as i64),
            Metric::new("nodata", self.nodata.get() as i64),
            Metric::new("referrals", self.referrals.get() as i64),
            Metric::new("refused", self.refused.get() as i64),
            Metric::new("forwarded", self.forwarded.get() as i64),
            Metric::new("servfail", self.servfail.get() as i64),
            Metric::new("records", self.zones.iter().map(|z| z.records.len()).sum::<usize>() as i64),
        ]
    }
    fn status(&self) -> String {
        let names: Vec<&str> = self.zones.iter().map(|z| if z.origin.is_empty() { "." } else { z.origin.as_str() }).collect();
        format!("Zonen: {}{}", if names.is_empty() { String::from("keine") } else { names.join(", ") }, if self.forward { "; leitet weiter" } else { "" })
    }
    fn clone_box(&self) -> Box<dyn Service> {
        Box::new(self.clone())
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

impl Engine {
    /// the zones of a device's DNS server
    pub fn dns_zones(&self, dev: u32) -> Vec<Zone> {
        self.service_of::<DnsServer>(dev).map(|s| s.zones.clone()).unwrap_or_default()
    }
}
