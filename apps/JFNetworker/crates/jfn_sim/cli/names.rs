// names -- names at the command lines: dig, host and nslookup on the Linux shell (console
// programs that ask a name server themselves, as BIND 9's tools do: no cache, dig without the
// search list), "ip host", "ipv6 host", "ip name-server" and the ping of a name on the
// IOS-like console
//
//   dig [@SERVER] [-t TYPE] [-x ADDR] [TYPE] [NAME] [+short]   one question (A; NS of the root
//                        without a name; PTR with -x) with EDNS (1232 bytes) to SERVER or the
//                        device's first name server: the answer as dig prints it (header,
//                        flags, the sections in zone-file form in BIND's columns, query time,
//                        server, when, size), +short the data alone; exit status 9 when no
//                        server answers
//   host [-t TYPE] NAME|ADDR [SERVER]   A, AAAA and MX of the first name of the search list
//                        that exists (the one type with -t, PTR for an address): "has address",
//                        "has IPv6 address", "is an alias for", "mail is handled by", "domain
//                        name pointer"; "Host NAME not found: 3(NXDOMAIN)" (exit status 1)
//   nslookup [-type=TYPE] NAME|ADDR [SERVER]   A and AAAA through the search list (PTR for an
//                        address): "Server:", "Address:", "Non-authoritative answer:", "Name:",
//                        "Address:", "canonical name ="; "** server can't find NAME: NXDOMAIN"
//                        (exit status 1)
//   ip host NAME A.B.C.D, ipv6 host NAME X:X::X (IOS-like)   static names of either family
//                        (Action::HostEntry / HostEntry6; "no" removes the name's entries of
//                        that family)
//   ip name-server A [B [C]] (IOS-like)   adds name servers of either family to the list
//                        (Action::NameServers); "no ip name-server A" removes one, without an
//                        address all; the lines of the running configuration
//   ping NAME, ping ipv6 NAME (IOS-like)   the host table, else the name servers ("ip domain
//                        lookup"): ping NAME takes the name's IPv4 address, its IPv6 one only
//                        when it has none (jobs.rs: then6), ping ipv6 NAME asks for IPv6 alone;
//                        a source given with a name applies to IPv4
//
// The tools send from a socket of the server's family connected to it, so an ICMP port
// unreachable ends a try at once (";; communications error to S#53: connection refused"); a
// try waits 5 s, dig tries three times, host and nslookup twice.  Without a name server they
// ask 127.0.0.1, as BIND's tools do.  The first line of dig, the help texts and the IOS-like
// status lines are our own words; the layout of the answers follows the tools.

use alloc::boxed::Box;
use alloc::collections::VecDeque;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::any::Any;

use base::cmdtree::{ModeId, Parsed, Tree};
use netpkt::dns::{self, Message, RData, Rr, CLASS_IN, TYPE_A, TYPE_AAAA, TYPE_ANY, TYPE_CNAME, TYPE_MX, TYPE_NS, TYPE_OPT, TYPE_PTR};
use netpkt::udp::PORT_DNS;
use netpkt::{IpAddr, Ipv4, Ipv6};

use super::ios::K;
use super::print::JobOut;
use super::print6::up6;
use crate::action::Action;
use crate::jobs::PingOpts;
use crate::kernel::{Engine, Ns, MS, SEC};
use crate::os::{Metric, OsApi, Service, SvcEvent, ANY, ANY4};
use crate::resolver::{candidates, Family, ResolvConf};
use crate::tcp::TcpEnd;

/// the timer of a DNS tool: the wait for an answer
const K_WAIT: u32 = 0;
/// the buffer size dig offers with EDNS (BIND 9.18)
const EDNS_UDP: u16 = 1232;
/// how long a try waits
const WAIT: Ns = 5 * SEC;
/// a name that leads nowhere on the IOS-like console
const UNKNOWN: &str = "% Unrecognized host or address, or protocol not running.\n\n";

const DIG_USAGE: &str = "Usage:  dig [@server] [-t type] [-x addr] [type] [name] [+short]\n";
const DIG_HELP: &str = "Usage:  dig [@server] [-t type] [-x addr] [type] [name] [+short]\n  @server  the name server to ask (an address; else the device's first one)\n  -t type  the record type (A if not given: A, AAAA, CNAME, MX, NS, PTR, SOA, TXT, ANY)\n  -x addr  the name of an address (PTR under in-addr.arpa or ip6.arpa)\n  +short   only the data of the answer\nOne question with EDNS; three tries of 5 s each.\n";
const HOST_USAGE: &str = "Usage: host [-t type] {name|address} [server]\n";
const NSLOOKUP_USAGE: &str = "Usage: nslookup [-type=TYPE] {name|address} [server] (no interactive mode here)\n";

// ---------------------------------------------------------------- the IOS-like commands

/// the IOS-like commands of names in the global configuration
pub(crate) fn build_names(t: &mut Tree<K>, conf: ModeId) {
    t.add(conf, "ip host", "Give an IPv4 address a name for this device", None, 0);
    t.add(conf, "ip host WORD=hname", "The name", None, 0);
    t.add(conf, "ip host WORD=hname A.B.C.D=haddr", "Its IPv4 address", Some(K::IpHost), 0);
    t.add_no(conf, "ip host WORD=hname", K::IpHost);
    t.add(conf, "ipv6 host", "Give an IPv6 address a name for this device", None, 0);
    t.add(conf, "ipv6 host WORD=hname", "The name", None, 0);
    t.add(conf, "ipv6 host WORD=hname X:X:X:X::X=haddr6", "Its IPv6 address", Some(K::Ipv6Host), 0);
    t.add_no(conf, "ipv6 host WORD=hname", K::Ipv6Host);
    t.add(conf, "ip name-server", "Name servers this device asks", None, 0);
    t.add_no(conf, "ip name-server", K::NameServer);
    // up to three addresses, each of either family
    let mut level = alloc::vec![String::from("ip name-server")];
    for (key, help) in [("ns1", "Address of a name server"), ("ns2", "Address of one more"), ("ns3", "Address of one more")] {
        let mut next = Vec::new();
        for at in &level {
            for kind in ["A.B.C.D", "X:X:X:X::X"] {
                let path = format!("{} {}={}", at, kind, key);
                t.add(conf, &path, help, Some(K::NameServer), 0);
                next.push(path);
            }
        }
        level = next;
    }
}

/// an address as IOS writes it (IPv6 in capitals)
fn ios_ip(a: IpAddr) -> String {
    match a {
        IpAddr::V4(x) => format!("{}", x),
        IpAddr::V6(x) => up6(x),
    }
}

impl Engine {
    /// "ip host", "ipv6 host", "ip name-server" and their no-forms
    pub(crate) fn ios_names(&mut self, ni: usize, p: &Parsed<K>) -> Result<(), String> {
        let dev = self.dev_of(ni);
        let name = String::from(p.text("hname").unwrap_or(""));
        match p.cmd {
            K::IpHost => {
                let ip = if p.no { None } else { p.ip("haddr").map(Ipv4) };
                let _ = self.apply(Action::HostEntry { dev, name, ip });
            }
            K::Ipv6Host => {
                let ip = if p.no { None } else { p.ip6("haddr6").map(Ipv6) };
                let _ = self.apply(Action::HostEntry6 { dev, name, ip });
            }
            K::NameServer => {
                let given: Vec<IpAddr> = ["ns1", "ns2", "ns3"].iter().filter_map(|k| p.ip(k).map(|a| IpAddr::V4(Ipv4(a))).or_else(|| p.ip6(k).map(|a| IpAddr::V6(Ipv6(a))))).collect();
                if let Some(bad) = given.iter().find(|a| a.is_unspecified() || a.is_multicast()) {
                    return self.ios_err(ni, &format!("% Invalid name server address {}\n", ios_ip(*bad)));
                }
                let mut servers = self.os(ni).map(|o| o.resolver.statics.servers.clone()).unwrap_or_default();
                if p.no && given.is_empty() {
                    servers.clear();
                } else if p.no {
                    servers.retain(|s| !given.contains(s));
                } else {
                    for a in given {
                        if !servers.contains(&a) {
                            servers.push(a);
                        }
                    }
                }
                let _ = self.apply(Action::NameServers { dev, servers });
            }
            _ => {}
        }
        Ok(())
    }

    /// the lines of the running configuration: static names of both families, the name
    /// servers set by hand
    pub(crate) fn ios_names_config(&self, ni: usize) -> String {
        let Some(os) = self.os(ni) else {
            return String::new();
        };
        let mut s = String::new();
        for (h, a) in &os.hosts {
            if let IpAddr::V4(x) = a {
                s.push_str(&format!("ip host {} {}\n", h, x));
            }
        }
        for (h, a) in &os.hosts {
            if let IpAddr::V6(x) = a {
                s.push_str(&format!("ipv6 host {} {}\n", h, up6(*x)));
            }
        }
        let servers = &os.resolver.statics.servers;
        if !servers.is_empty() {
            let list: Vec<String> = servers.iter().map(|a| ios_ip(*a)).collect();
            s.push_str(&format!("ip name-server {}\n", list.join(" ")));
        }
        if !s.is_empty() {
            s.push_str("!\n");
        }
        s
    }

    /// IOS-like "ping NAME" (`v6`: "ping ipv6 NAME"), see the header
    pub(crate) fn ios_ping_name(&mut self, ni: usize, p: &Parsed<K>, name: &str, v6: bool) -> Result<(), String> {
        let dev = self.dev_of(ni);
        let src = if v6 { None } else { self.ios_ping_src(ni, p)? };
        // the host table first: an IPv4 entry, else an IPv6 one
        let entry = |four: bool| self.os(ni).is_some_and(|o| o.hosts.iter().any(|(h, a)| h.eq_ignore_ascii_case(name) && a.is_v4() == four));
        let (h4, h6) = (entry(true), entry(false));
        let hosted = if v6 { h6 } else { h4 || h6 };
        if !hosted {
            let lookup = self.cdev(ni).is_some_and(|c| c.cfg.domain_lookup);
            let server = self.resolv_conf(dev).and_then(|c| c.servers.first().copied());
            match server {
                _ if !lookup => return self.ios_err(ni, UNKNOWN),
                // nobody to ask (IOS would broadcast its question)
                None if v6 => return self.ios_err(ni, UNKNOWN),
                None => return self.ios_err(ni, &format!("Resolving \"{}\" via DNS (255.255.255.255) ...\n{}", name, UNKNOWN)),
                Some(s) => self.out(ni, &format!("Resolving \"{}\" via DNS ({}) ...\n", name, ios_ip(s))),
            }
        }
        let count = p.num("count").unwrap_or(5) as u32;
        let bytes = p.num("bytes").unwrap_or(100) as u16;
        let timeout = p.num("secs").unwrap_or(2) as Ns * SEC;
        let df = p.has("df-bit");
        // the same datagram size over either family (48 bytes of IPv6 and ICMPv6 headers)
        let size6 = bytes.max(48) - 48;
        let six = v6 || (h6 && !h4);
        let (fam, size) = if six { (Family::V6, size6) } else { (Family::V4, bytes.saturating_sub(28)) };
        let o = PingOpts { count, size, ttl: None, df, interval: 0, timeout, src };
        // a name the name servers know: IPv6 when it has no IPv4 address
        let Ok(id) = self.ping_host_fam(dev, name, o, fam, !six && !hosted, Some(size6)) else {
            return Ok(());
        };
        self.cli_start_job(ni, id, JobOut::IosByName { count, bytes, timeout, df });
        Ok(())
    }

    // ------------------------------------------------------------ the Linux tools

    /// "dig ..."
    pub(crate) fn sh_dig(&mut self, ni: usize, args: &[&str]) -> Result<(), String> {
        let mut server: Option<&str> = None;
        let mut qtype: Option<u16> = None;
        let mut name: Option<String> = None;
        let mut short = false;
        let mut k = 0;
        while k < args.len() {
            let a = args[k];
            k += 1;
            match a {
                "-h" => {
                    self.out(ni, DIG_HELP);
                    return Ok(());
                }
                // the family of the transport: the server's address decides it here
                "-4" | "-6" => {}
                "-t" | "-x" => {
                    let Some(v) = args.get(k) else {
                        return self.sh_err(ni, &format!("dig: option {} needs a value\n{}", a, DIG_USAGE));
                    };
                    k += 1;
                    if a == "-t" {
                        let Some(t) = dns::type_by_name(v) else {
                            return self.sh_err(ni, &format!("dig: invalid type: {}\n", v));
                        };
                        qtype = Some(t);
                    } else {
                        let Some(ip) = IpAddr::parse(v) else {
                            return self.sh_err(ni, &format!("dig: '{}' is not an IP address\n", v));
                        };
                        name = Some(reverse_name(ip));
                        qtype = qtype.or(Some(TYPE_PTR));
                    }
                }
                _ if a.starts_with('@') => server = Some(&a[1..]),
                _ if a.starts_with('+') => match &a[1..] {
                    "short" => short = true,
                    "noshort" => short = false,
                    _ => {}
                },
                _ if a.starts_with('-') => return self.sh_err(ni, &format!("Invalid option: {}\n{}", a, DIG_USAGE)),
                _ if a.eq_ignore_ascii_case("IN") => {}
                _ if qtype.is_none() && dns::type_by_name(a).is_some() => qtype = dns::type_by_name(a),
                _ if name.is_none() => name = Some(String::from(a)),
                _ => {}
            }
        }
        let (name, qtype) = match name {
            Some(n) => (n, qtype.unwrap_or(TYPE_A)),
            // without a name: the name servers of the root
            None => (String::from("."), qtype.unwrap_or(TYPE_NS)),
        };
        let banner = format!("; dig of the JFNetworker shell: {}\n", args.join(" "));
        let tool = DnsTool::new(Tool::Dig { banner, short }, alloc::vec![(name, qtype)]);
        self.dns_tool(ni, "dig", tool, server, false)
    }

    /// "host [-t TYPE] NAME|ADDR [SERVER]"
    pub(crate) fn sh_host(&mut self, ni: usize, args: &[&str]) -> Result<(), String> {
        let mut qtype: Option<u16> = None;
        let mut pos: Vec<&str> = Vec::new();
        let mut k = 0;
        while k < args.len() {
            let a = args[k];
            k += 1;
            if a == "-t" {
                let Some(v) = args.get(k) else {
                    return self.sh_err(ni, HOST_USAGE);
                };
                k += 1;
                let Some(t) = dns::type_by_name(v) else {
                    return self.sh_err(ni, &format!("host: invalid type: {}\n", v));
                };
                qtype = Some(t);
            } else if a.len() > 1 && a.starts_with('-') {
                // -4, -6, -v and the like change nothing here
            } else {
                pos.push(a);
            }
        }
        let Some(&name) = pos.first() else {
            return self.sh_err(ni, HOST_USAGE);
        };
        let qs = match (IpAddr::parse(name), qtype) {
            (Some(ip), None) => alloc::vec![(reverse_name(ip), TYPE_PTR)],
            (_, Some(t)) => alloc::vec![(String::from(name), t)],
            (None, None) => [TYPE_A, TYPE_AAAA, TYPE_MX].iter().map(|&t| (String::from(name), t)).collect(),
        };
        let tool = DnsTool::new(Tool::Host { explicit: qtype.is_some() }, qs);
        self.dns_tool(ni, "host", tool, pos.get(1).copied(), true)
    }

    /// "nslookup [-type=TYPE] NAME|ADDR [SERVER]"
    pub(crate) fn sh_nslookup(&mut self, ni: usize, args: &[&str]) -> Result<(), String> {
        let mut qtype: Option<u16> = None;
        let mut pos: Vec<&str> = Vec::new();
        for &a in args {
            match a.split_once('=') {
                Some((o, v)) if ["-type", "-query", "-q", "-t"].contains(&o) => match dns::type_by_name(v) {
                    Some(t) => qtype = Some(t),
                    None => return self.sh_err(ni, &format!("*** Invalid option: {}\n", &a[1..])),
                },
                _ if a.starts_with('-') => {}
                _ => pos.push(a),
            }
        }
        let Some(&name) = pos.first() else {
            return self.sh_err(ni, NSLOOKUP_USAGE);
        };
        let qs = match (IpAddr::parse(name), qtype) {
            (Some(ip), None) => alloc::vec![(reverse_name(ip), TYPE_PTR)],
            (_, Some(t)) => alloc::vec![(String::from(name), t)],
            (None, None) => alloc::vec![(String::from(name), TYPE_A), (String::from(name), TYPE_AAAA)],
        };
        let tool = DnsTool::new(Tool::Nslookup, qs);
        self.dns_tool(ni, "nslookup", tool, pos.get(1).copied(), true)
    }

    /// starts a DNS tool on the console: its server is the one given, else the device's first,
    /// else 127.0.0.1; `search`: it goes through the search list
    fn dns_tool(&mut self, ni: usize, prog: &str, mut tool: DnsTool, server: Option<&str>, search: bool) -> Result<(), String> {
        let dev = self.dev_of(ni);
        let Some(mut conf) = self.resolv_conf(dev) else {
            return self.sh_err(ni, &format!("{}: this device has no network stack\n", prog));
        };
        tool.server = match server {
            Some(s) => match IpAddr::parse(s.trim_start_matches('[').trim_end_matches(']')) {
                Some(a) => a,
                None => return self.sh_err(ni, &format!("{}: couldn't get address for '{}': not found\n", prog, s)),
            },
            None => conf.servers.first().copied().unwrap_or(IpAddr::V4(Ipv4::LOCALHOST)),
        };
        tool.given = server.map(String::from);
        if !search {
            conf.search.clear();
        }
        tool.conf = conf;
        self.run_program(ni, prog, Box::new(tool), false)
    }
}

// ---------------------------------------------------------------- the console program

/// which tool it is and how it prints
#[derive(Clone, Debug, PartialEq, Eq)]
enum Tool {
    /// `banner`: our first line (with the words typed); `short`: +short
    Dig { banner: String, short: bool },
    /// `explicit`: a type was given, so its absence is said
    Host { explicit: bool },
    Nslookup,
}

impl Tool {
    /// tries per question
    fn tries(&self) -> u8 {
        if let Tool::Dig { .. } = self { 3 } else { 2 }
    }
    /// the exit status when no server answers
    fn fail_exit(&self) -> i32 {
        if let Tool::Dig { .. } = self { 9 } else { 1 }
    }
}

/// the name asked for now
#[derive(Clone, Debug)]
struct Asked {
    name: String,
    qtype: u16,
    id: u16,
    sent: Ns,
    tries: u8,
}

/// dig, host or nslookup on the console: its questions one after the other to one server
#[derive(Clone, Debug)]
pub struct DnsTool {
    tool: Tool,
    /// the questions still to ask: (name as typed, type)
    queue: VecDeque<(String, u16)>,
    /// the question asked now
    q: (String, u16),
    server: IpAddr,
    /// the server as typed, when it was given (host says "Using domain server:")
    given: Option<String>,
    /// the search list (empty for dig)
    conf: ResolvConf,
    /// the names still to try for the question
    cands: VecDeque<String>,
    cur: Option<Asked>,
    port: u16,
    /// host and nslookup: the first lines are out
    head: bool,
    /// host and nslookup: the owners whose CNAME was said
    aliases: Vec<String>,
    /// host and nslookup: the name that exists (the next questions ask it alone)
    found: Option<String>,
    /// nslookup: some data came ("*** Can't find NAME: No answer" otherwise)
    data: bool,
    exit: i32,
}

impl DnsTool {
    fn new(tool: Tool, qs: Vec<(String, u16)>) -> DnsTool {
        DnsTool {
            tool,
            queue: qs.into_iter().collect(),
            q: (String::new(), TYPE_A),
            server: IpAddr::V4(Ipv4::LOCALHOST),
            given: None,
            conf: ResolvConf { servers: Vec::new(), search: Vec::new(), ndots: 1, timeout: WAIT, attempts: 2 },
            cands: VecDeque::new(),
            cur: None,
            port: 0,
            head: false,
            aliases: Vec::new(),
            found: None,
            data: false,
            exit: 0,
        }
    }

    /// the next name to try, the next question, or the end
    fn next(&mut self, os: &mut OsApi) {
        if self.cands.is_empty() {
            let Some(q) = self.queue.pop_front() else {
                self.finish(os);
                return;
            };
            let names = match &self.found {
                Some(f) => alloc::vec![f.clone()],
                None => candidates(&q.0, &self.conf),
            };
            self.cands = names.into_iter().collect();
            self.q = q;
        }
        let Some(name) = self.cands.pop_front() else {
            self.finish(os);
            return;
        };
        let id = os.rand() as u16;
        self.cur = Some(Asked { name, qtype: self.q.1, id, sent: 0, tries: 0 });
        self.send(os);
    }

    /// asks the current name (again, with the same id)
    fn send(&mut self, os: &mut OsApi) {
        let now = os.now();
        let edns = matches!(self.tool, Tool::Dig { .. });
        let Some(a) = self.cur.as_mut() else {
            return;
        };
        a.sent = now;
        a.tries += 1;
        let mut m = Message::query(a.id, &a.name, a.qtype, true);
        if edns {
            m.additional.push(Rr { name: String::new(), rtype: TYPE_OPT, class: EDNS_UDP, ttl: 0, data: RData::Other(Vec::new()) });
        }
        let name = shown(&a.name);
        if os.send(self.port, self.server, PORT_DNS, &m.to_bytes()) {
            os.timer(K_WAIT, now + WAIT);
            return;
        }
        os.print(&format!(";; UDP setup with {s}#53({s}) for {n} failed: network unreachable.\n", s = self.server, n = name));
        self.give_up(os);
    }

    /// the answer to the current name
    fn reply(&mut self, os: &mut OsApi, m: Message, size: usize) {
        os.cancel(K_WAIT);
        let Some(a) = self.cur.take() else {
            return;
        };
        let rcode = m.header.rcode;
        // host and nslookup: a name of the search list that does not exist, the next one
        if !matches!(self.tool, Tool::Dig { .. }) && rcode == dns::RCODE_NXDOMAIN && !self.cands.is_empty() {
            self.next(os);
            return;
        }
        // the question is settled: the rest of the search list is not asked
        self.cands.clear();
        let typed = self.q.0.clone();
        let mut s = String::new();
        match self.tool.clone() {
            Tool::Dig { banner, short } => {
                s = if short { dig_short(&m) } else { dig_text(&banner, &m, os.now().saturating_sub(a.sent), self.server, size, os.clock()) };
            }
            Tool::Host { explicit } => {
                if let Some(g) = self.given.as_ref().filter(|_| !self.head) {
                    s.push_str(&format!("Using domain server:\nName: {}\nAddress: {}#53\nAliases: \n\n", g, self.server));
                }
                self.head = true;
                if rcode == dns::RCODE_NOERROR {
                    self.found = Some(a.name.clone());
                    let lines = self.host_lines(&m, a.qtype, explicit, &a.name);
                    s.push_str(&lines);
                } else {
                    s.push_str(&format!("Host {} not found: {}({})\n", typed, rcode, dns::rcode_name(rcode)));
                    self.exit = 1;
                    self.queue.clear();
                }
            }
            Tool::Nslookup => {
                if !self.head {
                    s.push_str(&format!("Server:\t\t{}\nAddress:\t{}#53\n\n", self.server, self.server));
                    self.head = true;
                }
                if rcode == dns::RCODE_NOERROR {
                    if self.found.is_none() && !m.header.aa {
                        s.push_str("Non-authoritative answer:\n");
                    }
                    self.found = Some(a.name.clone());
                    let lines = self.ns_lines(&m);
                    s.push_str(&lines);
                } else {
                    s.push_str(&format!("** server can't find {}: {}\n", shown(&typed), dns::rcode_name(rcode)));
                    self.exit = 1;
                    self.queue.clear();
                }
            }
        }
        os.print(&s);
        self.next(os);
    }

    /// a try ended without an answer (`why`: "timed out", "connection refused"): again, or
    /// the end
    fn failed(&mut self, os: &mut OsApi, why: &str) {
        os.cancel(K_WAIT);
        os.print(&format!(";; communications error to {}#53: {}\n", self.server, why));
        if self.cur.as_ref().is_some_and(|a| a.tries < self.tool.tries()) {
            self.send(os);
        } else {
            self.give_up(os);
        }
    }

    /// no server answered: the tool ends
    fn give_up(&mut self, os: &mut OsApi) {
        os.cancel(K_WAIT);
        let text = match &self.tool {
            Tool::Dig { banner, short: false } => format!("\n{};; no servers could be reached\n", banner),
            _ => String::from(";; no servers could be reached\n"),
        };
        os.print(&text);
        self.cur = None;
        self.queue.clear();
        self.cands.clear();
        os.exit(self.tool.fail_exit());
    }

    /// every question was answered
    fn finish(&mut self, os: &mut OsApi) {
        self.cur = None;
        if self.tool == Tool::Nslookup {
            if self.exit == 0 && !self.data {
                os.print(&format!("*** Can't find {}: No answer\n", shown(&self.q.0)));
            }
            os.print("\n");
        }
        os.exit(self.exit);
    }

    /// true the first time the CNAME of `owner` is to be said
    fn say_alias(&mut self, owner: &str) -> bool {
        let k = owner.to_ascii_lowercase();
        if self.aliases.contains(&k) {
            return false;
        }
        self.aliases.push(k);
        true
    }

    /// host's lines for an answer to `qtype`
    fn host_lines(&mut self, m: &Message, qtype: u16, explicit: bool, name: &str) -> String {
        let mut s = String::new();
        let mut hit = false;
        for r in &m.answers {
            let owner = shown(&r.name);
            if let RData::Cname(t) = &r.data
                && qtype != TYPE_CNAME
            {
                if self.say_alias(&r.name) {
                    s.push_str(&format!("{} is an alias for {}\n", owner, abs(t)));
                }
                continue;
            }
            if r.rtype != qtype && qtype != TYPE_ANY {
                continue;
            }
            hit = true;
            let line = match &r.data {
                RData::A(x) => format!("{} has address {}", owner, x),
                RData::Aaaa(x) => format!("{} has IPv6 address {}", owner, x),
                RData::Mx { pref, host } => format!("{} mail is handled by {} {}", owner, pref, abs(host)),
                RData::Ptr(t) => format!("{} domain name pointer {}", owner, abs(t)),
                RData::Ns(t) => format!("{} name server {}", owner, abs(t)),
                RData::Cname(t) => format!("{} is an alias for {}", owner, abs(t)),
                RData::Txt(_) => format!("{} descriptive text {}", owner, r.data.text()),
                d => format!("{} has {} record {}", owner, dns::type_name(r.rtype), dig_data(d)),
            };
            s.push_str(&line);
            s.push('\n');
        }
        if !hit && explicit {
            s.push_str(&format!("{} has no {} record\n", shown(name), dns::type_name(qtype)));
        }
        self.data |= hit;
        s
    }

    /// nslookup's lines for an answer
    fn ns_lines(&mut self, m: &Message) -> String {
        let mut s = String::new();
        for r in &m.answers {
            let owner = shown(&r.name);
            match &r.data {
                RData::Cname(t) => {
                    if self.say_alias(&r.name) {
                        s.push_str(&format!("{}\tcanonical name = {}\n", owner, abs(t)));
                    }
                    continue;
                }
                RData::A(x) => s.push_str(&format!("Name:\t{}\nAddress: {}\n", owner, x)),
                RData::Aaaa(x) => s.push_str(&format!("Name:\t{}\nAddress: {}\n", owner, x)),
                RData::Ptr(t) => s.push_str(&format!("{}\tname = {}\n", owner, abs(t))),
                RData::Mx { pref, host } => s.push_str(&format!("{}\tmail exchanger = {} {}\n", owner, pref, abs(host))),
                RData::Ns(t) => s.push_str(&format!("{}\tnameserver = {}\n", owner, abs(t))),
                RData::Txt(_) => s.push_str(&format!("{}\ttext = {}\n", owner, r.data.text())),
                _ => continue,
            }
            self.data = true;
        }
        s
    }
}

/// a name as the tools print an owner: without the final dot, the root as "."
fn shown(name: &str) -> String {
    let n = name.trim_end_matches('.');
    String::from(if n.is_empty() { "." } else { n })
}

/// a name with its final dot ("www.lab.", the root ".")
fn abs(name: &str) -> String {
    let n = name.trim_end_matches('.');
    if n.is_empty() { String::from(".") } else { format!("{}.", n) }
}

/// the PTR name of an address (in-addr.arpa; ip6.arpa by nibbles, RFC 3596 2.5), absolute
fn reverse_name(ip: IpAddr) -> String {
    match ip {
        IpAddr::V4(a) => format!("{}.", dns::reverse_v4(a)),
        IpAddr::V6(a) => {
            let mut s = String::new();
            for b in a.0.iter().rev() {
                s.push_str(&format!("{:x}.{:x}.", b & 15, b >> 4));
            }
            s.push_str("ip6.arpa.");
            s
        }
    }
}

/// the class as dig writes it
fn class_name(c: u16) -> String {
    if c == CLASS_IN { String::from("IN") } else { format!("CLASS{}", c) }
}

/// the data of a record as dig writes it (names absolute)
fn dig_data(d: &RData) -> String {
    match d {
        RData::Ns(t) | RData::Cname(t) | RData::Ptr(t) => abs(t),
        RData::Mx { pref, host } => format!("{} {}", pref, abs(host)),
        RData::Soa(x) => format!("{} {} {} {} {} {} {}", abs(&x.mname), abs(&x.rname), x.serial, x.refresh, x.retry, x.expire, x.minimum),
        d => d.text(),
    }
}

/// dig +short: the data of the answer section
fn dig_short(m: &Message) -> String {
    let mut s = String::new();
    for r in &m.answers {
        s.push_str(&dig_data(&r.data));
        s.push('\n');
    }
    s
}

/// BIND's indent: tabs (8 columns) and spaces up to column `to`, at least one column on
fn indent(s: &mut String, col: &mut usize, to: usize) {
    let to = to.max(*col + 1);
    let tabs = to / 8 - *col / 8;
    let mut from = *col;
    if tabs > 0 {
        s.extend(core::iter::repeat_n('\t', tabs));
        from = to / 8 * 8;
    }
    s.extend(core::iter::repeat_n(' ', to - from));
    *col = to;
}

/// a record in dig's columns: TTL at 24, class at 32, type at 40, data at 48
fn dig_rr(r: &Rr) -> String {
    let mut s = abs(&r.name);
    let mut col = s.chars().count();
    for (to, part) in [(24, format!("{}", r.ttl)), (32, class_name(r.class)), (40, dns::type_name(r.rtype)), (48, dig_data(&r.data))] {
        indent(&mut s, &mut col, to);
        col += part.chars().count();
        s.push_str(&part);
    }
    s.push('\n');
    s
}

/// the whole answer as dig prints it
fn dig_text(banner: &str, m: &Message, rtt: Ns, server: IpAddr, size: usize, clock: i64) -> String {
    let h = &m.header;
    let mut s = format!("\n{}", banner);
    s.push_str(&format!(";; ->>HEADER<<- opcode: QUERY, status: {}, id: {}\n", dns::rcode_name(h.rcode), h.id));
    let flags: Vec<&str> = [(h.qr, "qr"), (h.aa, "aa"), (h.tc, "tc"), (h.rd, "rd"), (h.ra, "ra"), (h.ad, "ad"), (h.cd, "cd")].iter().filter(|f| f.0).map(|f| f.1).collect();
    s.push_str(&format!(";; flags: {}; QUERY: {}, ANSWER: {}, AUTHORITY: {}, ADDITIONAL: {}\n", flags.join(" "), m.questions.len(), m.answers.len(), m.authority.len(), m.additional.len()));
    if h.rd && !h.ra {
        s.push_str(";; WARNING: recursion requested but not available\n");
    }
    s.push('\n');
    if let Some(o) = m.additional.iter().find(|r| r.rtype == TYPE_OPT) {
        // the OPT record's TTL holds the EDNS version and the DO flag (RFC 6891 6.1.3)
        let dnssec = if o.ttl & 0x8000 != 0 { " do" } else { "" };
        s.push_str(&format!(";; OPT PSEUDOSECTION:\n; EDNS: version: {}, flags:{}; udp: {}\n", (o.ttl >> 16) & 0xFF, dnssec, o.class));
    }
    s.push_str(";; QUESTION SECTION:\n");
    for q in &m.questions {
        // the columns count from after the ";"
        let name = abs(&q.name);
        let mut col = name.chars().count();
        let mut l = format!(";{}", name);
        for (to, part) in [(32, class_name(q.qclass)), (40, dns::type_name(q.qtype))] {
            indent(&mut l, &mut col, to);
            col += part.chars().count();
            l.push_str(&part);
        }
        s.push_str(&l);
        s.push('\n');
    }
    let extra: Vec<Rr> = m.additional.iter().filter(|r| r.rtype != TYPE_OPT).cloned().collect();
    for (title, sec) in [("ANSWER", &m.answers), ("AUTHORITY", &m.authority), ("ADDITIONAL", &extra)] {
        if sec.is_empty() {
            continue;
        }
        s.push_str(&format!("\n;; {} SECTION:\n", title));
        for r in sec.iter() {
            s.push_str(&dig_rr(r));
        }
    }
    const DAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let c = datetime::Civil::from_unix(clock.div_euclid(1_000_000_000));
    let when = format!("{} {} {:02} {:02}:{:02}:{:02} UTC {}", DAYS[c.weekday() as usize % 7], MONTHS[(c.month as usize).clamp(1, 12) - 1], c.day, c.hour, c.minute, c.second, c.year);
    s.push_str(&format!("\n;; Query time: {} msec\n;; SERVER: {}#53({}) (UDP)\n;; WHEN: {}\n;; MSG SIZE  rcvd: {}\n\n", rtt / MS, server, server, when, size));
    s
}

impl Service for DnsTool {
    fn kind(&self) -> &'static str {
        match self.tool {
            Tool::Dig { .. } => "dig",
            Tool::Host { .. } => "host",
            Tool::Nslookup => "nslookup",
        }
    }
    fn start(&mut self, os: &mut OsApi) {
        // a socket of the server's family, connected to it: ICMP errors end a try
        let bind = if self.server.is_v4() { ANY4 } else { ANY };
        let Ok(p) = os.bind_on(bind, 0, None) else {
            os.print(";; no servers could be reached\n");
            os.exit(self.tool.fail_exit());
            return;
        };
        self.port = p;
        os.udp_connect(p, Some((self.server, PORT_DNS)));
        self.next(os);
    }
    fn on(&mut self, ev: SvcEvent, os: &mut OsApi) {
        match ev {
            SvcEvent::Udp(d) if d.sport == PORT_DNS && d.src == self.server => {
                let Ok(m) = Message::parse(&d.data) else {
                    return;
                };
                let fits = self.cur.as_ref().is_some_and(|a| m.header.qr && m.header.id == a.id && m.questions.first().is_some_and(|q| q.qtype == a.qtype && dns::name_eq(&q.name, &a.name)));
                if fits {
                    self.reply(os, m, d.data.len());
                }
            }
            SvcEvent::UdpError { port, err, .. } if port == self.port && self.cur.is_some() => {
                self.failed(os, if matches!(err, TcpEnd::Refused) { "connection refused" } else { "host unreachable" });
            }
            SvcEvent::Timer(K_WAIT) if self.cur.is_some() => self.failed(os, "timed out"),
            _ => {}
        }
    }
    fn metrics(&self) -> Vec<Metric> {
        Vec::new()
    }
    fn status(&self) -> String {
        String::from(self.kind())
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
