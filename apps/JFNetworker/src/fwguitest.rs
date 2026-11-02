// fwguitest -- checks of the Firewall tab (part of --selftest, group "fwgui"): what its forms make for
// every kind of rule and for NAT, and that the device's console takes each of these lines; what they
// refuse and why; rules read back into rows (a list typed on the console, a raw line, remarks, the
// implicit deny) and back into the fields; moving a rule with its counters and remarks, on the engine;
// Acls::probe; the Test part on a table of packets, each sent for real as well, whose prediction must
// equal the engine's verdict (inbound, outbound, outbound after NAT); console::run_lines_running: a
// list typed and not saved, moved through the startup configuration, one undo step, a refusal that
// changes nothing, a device without routing
//
// The network (device ids in order): PC1 (192.168.1.10) and PC2 (.11) on SW1; R1 between
// 192.168.1.0/24 (Gi0/0/0) and 203.0.113.0/24 (Gi0/0/1); SRV (203.0.113.10) with TCP echo on port 80.

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use agent_core::jmap::Value;
use jfn_model::project::{PortRef, Project, ServiceDef};
use jfn_sim::acl::{Ace, Acl, AclKind, Acls, Item, Ports, Probe, Verdict, Wild};
use jfn_sim::nat::{Rule, Side, Target};
use jfn_sim::{Engine, Note, PduProto, PduSender, PduSpec, PingOpts, Report, MS, SEC};
use netpkt::ipv4::{PROTO_ICMP, PROTO_TCP, PROTO_UDP};
use netpkt::nat::Flow;
use netpkt::Ipv4;

use crate::doc::console;
use crate::selftest::T;
use crate::workspace::devwin::firewall::*;

const BUDGET: usize = 4_000_000;
const PC1: u32 = 1;
const PC2: u32 = 2;
const R1: u32 = 4;
const SRV: u32 = 5;

const R1_CFG: &str = "hostname R1\n!\ninterface GigabitEthernet0/0/0\n ip address 192.168.1.1 255.255.255.0\n no shutdown\n!\ninterface GigabitEthernet0/0/1\n ip address 203.0.113.1 255.255.255.0\n no shutdown\n!\nend\n";

fn s(x: &str) -> String {
    String::from(x)
}

fn ip(x: &str) -> Ipv4 {
    Ipv4::parse(x).unwrap_or(Ipv4::UNSPECIFIED)
}

fn pr(dev: u32, port: &str) -> PortRef {
    PortRef { dev, port: s(port) }
}

fn project() -> Project {
    let mut p = Project::new("firewall", 11);
    for m in ["pc", "pc", "jf-s24p", "jf-r4", "server-1u"] {
        let _ = p.add_device(m, 0, 0);
    }
    let _ = p.connect(&pr(PC1, "eth0"), &pr(3, "gi1/0/1"), None);
    let _ = p.connect(&pr(PC2, "eth0"), &pr(3, "gi1/0/2"), None);
    let _ = p.connect(&pr(3, "gi1/0/24"), &pr(R1, "gi0/0/0"), None);
    let _ = p.connect(&pr(R1, "gi0/0/1"), &pr(SRV, "eth0"), None);
    let host = |name: &str, a: &str, gw: &str| format!("hostname {}\nip addr add {} dev eth0\nip route add default via {}\n", name, a, gw);
    let cfgs = [(PC1, "PC1", host("PC1", "192.168.1.10/24", "192.168.1.1")), (PC2, "PC2", host("PC2", "192.168.1.11/24", "192.168.1.1")), (3, "SW1", s("hostname SW1\n!\nend\n")), (R1, "R1", s(R1_CFG)), (SRV, "SRV", host("SRV", "203.0.113.10/24", "203.0.113.1"))];
    for (id, name, c) in cfgs {
        if let Some(d) = p.device_mut(id) {
            d.name = s(name);
            d.startup_config = c;
        }
    }
    if let Some(d) = p.device_mut(SRV) {
        let config = agent_core::json::parse(b"{\"port\": 80}").unwrap_or(Value::Null);
        d.services.push(ServiceDef { kind: s("tcp-echo"), name: s("echo"), disabled: false, config });
    }
    p
}

fn engine() -> Engine {
    Engine::from_project(&project()).unwrap_or_else(|_| Engine::new(0))
}

// what a device printed since the last call
fn out(e: &mut Engine, dev: u32) -> String {
    let mut o = String::new();
    for r in e.take_reports() {
        if let Report::CliOut { dev: d, bytes, .. } = r
            && d == dev
        {
            o.push_str(&String::from_utf8_lossy(&bytes));
        }
    }
    o
}

fn run_for(e: &mut Engine, span: u64) {
    let t = e.now() + span;
    e.run_until(t, BUDGET);
}

// lines in the configuration mode of R1, as the forms' lines run: what R1 said
fn conf(e: &mut Engine, ls: &[String]) -> String {
    for l in ["enable", "configure terminal"] {
        let _ = e.cli_line(R1, l);
    }
    run_for(e, 10 * MS);
    let _ = out(e, R1);
    let mut o = String::new();
    for l in ls {
        let _ = e.cli_line(R1, l);
        run_for(e, 10 * MS);
        o.push_str(&out(e, R1));
    }
    let _ = e.cli_line(R1, "end");
    run_for(e, 10 * MS);
    let _ = out(e, R1);
    o
}

fn strs(ls: &[&str]) -> Vec<String> {
    ls.iter().map(|x| s(x)).collect()
}

fn acls_of(e: &Engine) -> Acls {
    e.node(R1).and_then(|x| x.ip.as_ref()).map(|st| st.acls.clone()).unwrap_or_default()
}

fn list_of(e: &Engine, name: &str) -> Acl {
    acls_of(e).lists.get(name).cloned().unwrap_or_else(|| Acl::new(AclKind::Extended, false))
}

// a list's lines as the configuration has them, with their counters
fn shown(a: &Acl) -> Vec<(String, u64)> {
    a.lines
        .iter()
        .map(|l| match &l.item {
            Item::Rule(r) => (r.config(a.kind), l.hits),
            Item::Remark(t) => (format!("remark {}", t), 0),
        })
        .collect()
}

fn texts(a: &Acl) -> Vec<String> {
    shown(a).into_iter().map(|x| x.0).collect()
}

// a PDU as a service of its own: its name
fn pdu(e: &mut Engine, dev: u32, proto: PduProto, dst: &str, sport: u16, dport: u16) -> String {
    let name = format!("pdu{}", e.now());
    let spec = PduSpec { proto, dst: s(dst), sport, dport, size: 20, ttl: None, count: 1, interval: MS };
    let _ = e.add_service(dev, &name, Box::new(PduSender::new(spec)), true);
    name
}

// three pings teach R1 and the hosts their neighbours
fn warm(e: &mut Engine) {
    let _ = e.ping(PC1, ip("203.0.113.10"), PingOpts { count: 3, ..PingOpts::default() });
    let _ = e.ping(PC2, ip("203.0.113.10"), PingOpts { count: 3, ..PingOpts::default() });
    run_for(e, 4 * SEC);
}

pub fn run(t: &mut T) {
    forms(t);
    refused_forms(t);
    rows(t);
    moves(t);
    probing(t);
    nat_forms(t);
    packets(t);
    steps(t);
}

fn tcp_form(f: impl FnOnce(&mut RuleForm)) -> RuleForm {
    let mut x = RuleForm::new();
    x.proto = 1;
    f(&mut x);
    x
}

// every kind of rule: the fields, the line, the console takes it and reads the same rule
fn forms(t: &mut T) {
    t.eq("protocols: the four with their own fields come first", protos().iter().take(4).map(|p| p.0).collect::<Vec<_>>(), vec!["ip", "tcp", "udp", "icmp"]);
    let echo = jfn_sim::acl::ICMP_TYPES.iter().position(|x| x.0 == "echo").unwrap_or(0) as i32 + 1;
    let gre = protos().iter().position(|p| p.0 == "gre").unwrap_or(0) as i32;
    let ext: Vec<(&str, RuleForm)> = vec![
        ("permit ip any any", RuleForm::new()),
        ("deny tcp host 192.168.1.11 any eq www", tcp_form(|x| {
            x.act = 1;
            x.src = s("192.168.1.11");
            x.dop = 1;
            x.dp1 = s("80");
        })),
        ("deny tcp host 192.168.1.11 any eq www", tcp_form(|x| {
            x.act = 1;
            x.src = s("host 192.168.1.11");
            x.dop = 1;
            x.dp1 = s("www");
        })),
        ("permit tcp any gt 1023 any established", tcp_form(|x| {
            x.sop = 3;
            x.sp1 = s("1023");
            x.established = 1;
        })),
        ("permit tcp any any neq telnet log", tcp_form(|x| {
            x.dop = 2;
            x.dp1 = s("23");
            x.log = 1;
        })),
        ("deny tcp any any lt 1024", tcp_form(|x| {
            x.act = 1;
            x.dop = 4;
            x.dp1 = s("1024");
        })),
        ("permit udp 10.0.0.0 0.0.0.255 10.1.0.0 0.0.255.255 range 1000 2000", {
            let mut x = RuleForm::new();
            x.proto = 2;
            x.src = s("10.0.0.0/24");
            x.dst = s("10.1.0.0 0.0.255.255");
            x.dop = 5;
            x.dp1 = s("2000");
            x.dp2 = s("1000");
            x
        }),
        ("permit udp any any eq domain", {
            let mut x = RuleForm::new();
            x.proto = 2;
            x.dop = 1;
            x.dp1 = s("domain");
            x
        }),
        ("permit icmp any any echo log", {
            let mut x = RuleForm::new();
            x.proto = 3;
            x.icmp = echo;
            x.log = 1;
            x
        }),
        ("deny gre any any", {
            let mut x = RuleForm::new();
            x.act = 1;
            x.proto = gre;
            x
        }),
        // ports of a protocol that has none are not read
        ("deny icmp any any", {
            let mut x = RuleForm::new();
            x.act = 1;
            x.proto = 3;
            x.dop = 1;
            x.dp1 = s("hugo");
            x
        }),
        ("permit 112 any any", {
            let mut x = RuleForm::new();
            x.use_raw = 1;
            x.raw = s(" permit 112 any any ");
            x
        }),
    ];
    let std: Vec<(&str, RuleForm)> = vec![
        ("permit any", RuleForm::new()),
        ("deny 10.0.0.0 0.0.0.255", {
            let mut x = RuleForm::new();
            x.act = 1;
            x.src = s("10.0.0.0/24");
            x
        }),
        ("permit 10.0.0.5 log", {
            let mut x = RuleForm::new();
            x.src = s("10.0.0.5");
            x.log = 1;
            x
        }),
        ("deny 10.0.0.0 0.255.0.255", {
            let mut x = RuleForm::new();
            x.act = 1;
            x.src = s("10.0.0.0 0.255.0.255");
            x
        }),
    ];
    for (kind, cases) in [(AclKind::Extended, ext), (AclKind::Standard, std)] {
        for (line, f) in cases {
            let r = rule_of_form(kind, &f, "R1");
            t.eq(&format!("form: {}", line), r.as_ref().map(|r| r.config(kind)).ok(), Some(s(line)));
            // the console's own reading of that line is the same rule
            t.eq(&format!("console reads: {}", line), parse_line(kind, line, "R1").ok(), r.ok());
        }
    }
    let mut e = engine();
    t.eq("a new numbered list", new_list_lines(" 110 ", 1, &Acls::default()), Ok((s("110"), strs(&["ip access-list extended 110"]))));
    t.eq("a new named list", new_list_lines("WEB-1", 0, &Acls::default()), Ok((s("WEB-1"), strs(&["ip access-list standard WEB-1"]))));
    let o = conf(&mut e, &strs(&["ip access-list extended WEB", "permit tcp any any eq www"]));
    t.eq("the console takes them", o, s(""));
    t.check("the list is there", list_of(&e, "WEB").has_rules());
}

// what the forms refuse, with the reason
fn refused_forms(t: &mut T) {
    let has = |r: Result<Wild, String>, word: &str| r.err().is_some_and(|x| x.contains(word));
    t.check("no address", has(wild_of("10.0.0.300"), "keine Adresse"));
    t.check("no address: empty", has(wild_of(" "), "keine Adresse"));
    t.check("no address: words", has(wild_of("mein netz"), "keine Adresse"));
    t.check("no address: bad prefix", has(wild_of("10.0.0.0/33"), "keine Adresse"));
    t.check("a net mask is said to be one", has(wild_of("10.0.0.0 255.255.255.0"), "0.0.0.255") && has(wild_of("10.0.0.0 255.255.255.0"), "10.0.0.0/24"));
    t.eq("addresses", (wild_of("any"), wild_of("10.0.0.5"), wild_of("host 10.0.0.5"), wild_of("0.0.0.0/0")), (Ok(Wild::ANY), Ok(Wild::host(ip("10.0.0.5"))), Ok(Wild::host(ip("10.0.0.5"))), Ok(Wild::ANY)));
    t.eq("an address's free bits go", wild_of("10.0.0.5/24"), Ok(Wild::new(ip("10.0.0.0"), ip("0.0.0.255"))));
    for (w, text) in [(Wild::ANY, "any"), (Wild::host(ip("10.0.0.5")), "10.0.0.5"), (Wild::new(ip("10.0.0.0"), ip("0.0.0.255")), "10.0.0.0/24"), (Wild::new(ip("10.0.0.0"), ip("0.255.0.255")), "10.0.0.0 0.255.0.255")] {
        t.eq(&format!("an address back: {}", text), wild_text(&w), s(text));
        t.eq(&format!("and read again: {}", text), wild_of(text), Ok(w));
    }
    t.check("no port", port_of(PROTO_TCP, "hugo").is_err() && port_of(PROTO_TCP, "70000").is_err() && port_of(PROTO_UDP, "www").is_err());
    t.eq("ports", (port_of(PROTO_TCP, "www"), port_of(PROTO_UDP, " 53 ")), (Ok(80), Ok(53)));
    let mut bad = tcp_form(|x| {
        x.dop = 1;
        x.dp1 = s("hugo");
    });
    t.check("the rule says why", rule_of_form(AclKind::Extended, &bad, "R1").err().is_some_and(|x| x.contains("hugo")));
    bad.dop = 0;
    bad.src = s("1.2.3");
    t.check("the rule says why: the source", rule_of_form(AclKind::Extended, &bad, "R1").err().is_some_and(|x| x.contains("1.2.3")));
    for (line, word) in [("no 10", "keine Regel"), ("exit", "keine Regel"), ("", "keine Regel"), ("permit tcp any any eq hugo", "lehnt"), ("permit banana", "lehnt"), ("deny 10.0.0.0 any", "lehnt")] {
        let mut f = RuleForm::new();
        f.use_raw = 1;
        f.raw = s(line);
        t.check(&format!("raw line refused: {}", line), rule_of_form(AclKind::Extended, &f, "R1").err().is_some_and(|x| x.contains(word)));
    }
    let mut std = RuleForm::new();
    std.use_raw = 1;
    std.raw = s("permit tcp any any");
    t.check("a standard list takes no protocol", rule_of_form(AclKind::Standard, &std, "R1").err().is_some_and(|x| x.contains("lehnt")));
    // lists
    let mut acls = Acls::default();
    acls.lists.insert(s("110"), Acl::new(AclKind::Extended, true));
    acls.lists.insert(s("1"), Acl::new(AclKind::Standard, true));
    acls.lists.insert(s("WEB"), Acl::new(AclKind::Extended, false));
    for (name, kind, word) in [("", 1, "Namen"), ("110", 0, "passt nicht"), ("50", 1, "passt nicht"), ("200", 1, "passt nicht"), ("3000", 0, "passt nicht"), ("2 x", 1, "kein Listenname"), ("9lives", 1, "kein Listenname"), ("WEB", 1, "gibt es schon"), ("110", 1, "gibt es schon")] {
        t.check(&format!("new list refused: {:?} {}", name, kind), new_list_lines(name, kind, &acls).err().is_some_and(|x| x.contains(word)));
    }
    let short = |_: u16| s("Gi0/0/0");
    t.eq("delete a numbered list", del_list_lines("110", &acls, &[], &short), Ok(strs(&["no access-list 110"])));
    t.eq("delete a named list", del_list_lines("WEB", &acls, &[], &short), Ok(strs(&["no ip access-list extended WEB"])));
    t.check("delete: no such list", del_list_lines("X", &acls, &[], &short).is_err());
    acls.bound.insert((0, true), s("WEB"));
    t.check("delete: refused while it filters", del_list_lines("WEB", &acls, &[], &short).err().is_some_and(|x| x.contains("Gi0/0/0") && x.contains("ausgehend")));
    let rule = Rule::List { acl: s("1"), to: Target::Iface(s("GigabitEthernet0/0/1")), overload: true };
    t.check("delete: refused while NAT reads it", del_list_lines("1", &acls, &[rule], &short).err().is_some_and(|x| x.contains("NAT")));
    // no routing, nothing changes
    t.check("refusal: no routing", refusal(false, None, "R1").is_some_and(|x| x.contains("R1") && x.contains("Routing einschalten")));
    t.eq("refusal: the outside reason first", refusal(false, Some("x"), "R1"), Some(s("x")));
    t.eq("refusal: none", refusal(true, None, "R1"), None);
    // a twin
    let mut a = Acl::new(AclKind::Extended, true);
    let r = Ace { permit: true, proto: Some(PROTO_TCP), src: Wild::ANY, sport: Ports::Any, dst: Wild::ANY, dport: Ports::Eq(80), established: false, icmp: None, log: false };
    let _ = a.add(None, Item::Rule(r.clone()));
    t.check("a rule that is there already", add_lines("110", &a, &r, "", None).err().is_some_and(|x| x.contains("Zeile 10")));
    let _ = a.add(None, Item::Rule(Ace { dport: Ports::Eq(81), ..r.clone() }));
    t.check("an edit into a rule that is there", edit_lines("110", &a, 20, &r, "").err().is_some_and(|x| x.contains("Zeile 10")));
    t.eq("an edit that changes nothing types nothing", edit_lines("110", &a, 10, &r, ""), Ok(Vec::new()));
}

// a list typed on the console in rows, and back in the fields
fn rows(t: &mut T) {
    let mut e = engine();
    let o = conf(&mut e, &strs(&[
        "access-list 120 remark Web zu",
        "access-list 120 deny tcp host 192.168.1.11 any eq 80 log",
        "access-list 120 permit udp 10.0.0.0 0.0.0.255 gt 1023 any range 53 60",
        "access-list 120 permit tcp any any established",
        "access-list 120 remark erst die eine",
        "access-list 120 remark dann die andere",
        "access-list 120 permit icmp any any echo-reply",
        "access-list 120 permit icmp any any 3 9",
        "access-list 120 permit 112 any any",
        "access-list 120 deny ip any host 10.0.0.5",
        "access-list 120 deny ip 10.0.0.0 255.0.0.0 any",
        "access-list 120 remark am Ende",
        "access-list 10 permit 10.0.0.0 0.0.0.255",
    ]));
    t.eq("typed", o, s(""));
    let a = list_of(&e, "120");
    let v = rows_of(&a);
    let cells = |r: &Row| (r.seq, r.act.clone(), r.proto.clone(), r.src.clone(), r.dst.clone(), r.port.clone(), r.log, r.remark.clone());
    t.eq("row: a rule with its remark", cells(&v[0]), (10, s("deny"), s("tcp"), s("192.168.1.11"), s("any"), s("eq www"), true, s("Web zu")));
    t.eq("row: ports of both sides", cells(&v[1]), (20, s("permit"), s("udp"), s("10.0.0.0/24"), s("any"), s("von gt 1023, range domain 60"), false, s("")));
    t.eq("row: established", v[2].port.clone(), s("established"));
    t.eq("row: an ICMP message, two remarks", (v[3].port.clone(), v[3].remark.clone()), (s("echo-reply"), s("erst die eine · dann die andere")));
    t.eq("row: a rule the form cannot hold shows as its line", (v[4].raw, v[4].src.clone(), v[4].dst.clone(), v[4].line.clone()), (true, s("permit icmp any any 3 9"), s(""), s("permit icmp any any 3 9")));
    t.eq("row: a protocol number too", (v[5].raw, v[5].src.clone()), (true, s("permit 112 any any")));
    t.eq("row: a net mask as wildcard too", (v[7].raw, v[7].line.clone()), (true, s("deny ip 0.0.0.0 255.0.0.0 any")));
    t.eq("row: a remark at the end", (v[8].seq, v[8].remark.clone(), v[8].hits, v[8].line.clone()), (0, s("am Ende"), None, s("remark am Ende")));
    t.check("row: the implicit deny is the last", v[9].implicit && v[9].act == "deny" && v[9].hits.is_none() && v.len() == 10);
    t.eq("row: the implicit deny's words", v[9].line.clone(), s("deny ip any any"));
    t.eq("row: hits start at 0", v[0].hits, Some(0));
    let mut keys: Vec<u64> = v.iter().map(|r| r.key).collect();
    keys.sort_unstable();
    keys.dedup();
    t.eq("row keys differ", keys.len(), v.len());
    t.eq("an empty list has no rows", rows_of(&Acl::new(AclKind::Extended, false)).len(), 0);
    // every rule back in the fields and again the same rule
    for (name, kind) in [("120", AclKind::Extended), ("10", AclKind::Standard)] {
        for l in &list_of(&e, name).lines {
            if let Item::Rule(r) = &l.item {
                let f = form_of(r, kind);
                t.eq(&format!("the fields hold: {}", r.config(kind)), f.use_raw != 0, !expressible(r));
                t.eq(&format!("and give back: {}", r.config(kind)), rule_of_form(kind, &f, "R1").ok(), Some(r.clone()));
            }
        }
    }
}

// a list on the engine, moved by what the buttons type
fn moves(t: &mut T) {
    let mut e = engine();
    warm(&mut e);
    let o = conf(&mut e, &strs(&[
        "ip access-list extended HITS",
        "permit udp any any eq 1001",
        "permit udp any any eq 1002",
        "permit udp any any eq 1003",
        "permit ip any any",
        "exit",
        "interface GigabitEthernet0/0/0",
        "ip access-group HITS in",
    ]));
    t.eq("made", o, s(""));
    for p in [1001, 1002, 1003] {
        let _ = pdu(&mut e, PC1, PduProto::Udp, "203.0.113.10", 40000 + p, p);
        run_for(&mut e, 100 * MS);
    }
    let hits = |e: &Engine| shown(&list_of(e, "HITS")).iter().map(|x| x.1).collect::<Vec<u64>>();
    t.eq("each rule counted", hits(&e)[..3].to_vec(), vec![1, 1, 1]);
    let (r1, r2, r3, r4) = (s("permit udp any any eq 1001"), s("permit udp any any eq 1002"), s("permit udp any any eq 1003"), s("permit ip any any"));
    let a = list_of(&e, "HITS");
    t.eq("first up: nothing to do", move_lines("HITS", &a, 10, true), None);
    t.eq("last down: nothing to do", move_lines("HITS", &a, 40, false), None);
    // one place up: the moved rule alone starts over, numbers between its neighbours
    let lines = move_lines("HITS", &a, 30, true);
    t.eq("up: the lines", lines.clone(), Some(strs(&["ip access-list extended HITS", "no 30", "15 permit udp any any eq 1003"])));
    t.eq("up: the console takes them", conf(&mut e, &lines.unwrap_or_default()), s(""));
    t.eq("up: the order", texts(&list_of(&e, "HITS")), vec![r1.clone(), r3.clone(), r2.clone(), r4.clone()]);
    t.eq("up: the counters", hits(&e)[..3].to_vec(), vec![1, 0, 1]);
    // numbers no longer 10, 20, ...: first the numbers
    let a = list_of(&e, "HITS");
    let lines = move_lines("HITS", &a, 15, true);
    t.eq("again: the lines", lines.clone(), Some(strs(&["ip access-list resequence HITS 10 10", "ip access-list extended HITS", "no 20", "5 permit udp any any eq 1003"])));
    t.eq("again: the console takes them", conf(&mut e, &lines.unwrap_or_default()), s(""));
    t.eq("again: the order", texts(&list_of(&e, "HITS")), vec![r3.clone(), r1.clone(), r2.clone(), r4.clone()]);
    t.eq("again: the others keep their counters", hits(&e)[..3].to_vec(), vec![0, 1, 1]);
    // down, to the end
    let a = list_of(&e, "HITS");
    let seqs: Vec<u32> = a.lines.iter().map(|l| l.seq).collect();
    let lines = move_lines("HITS", &a, seqs[2], false);
    t.eq("down: the console takes them", conf(&mut e, &lines.unwrap_or_default()), s(""));
    t.eq("down: the order", texts(&list_of(&e, "HITS")), vec![r3.clone(), r1.clone(), r4.clone(), r2.clone()]);
    t.eq("down: the counters", shown(&list_of(&e, "HITS")).iter().map(|x| x.1).collect::<Vec<u64>>()[..2].to_vec(), vec![0, 1]);
    // the button that clears the counters
    t.eq("clearing the counters", conf(&mut e, &strs(&["do clear access-list counters HITS"])), s(""));
    t.eq("cleared", shown(&list_of(&e, "HITS")).iter().map(|x| x.1).sum::<u64>(), 0);
    // a list goes: what the buttons type for one by number and one by name
    let _ = conf(&mut e, &strs(&["interface GigabitEthernet0/0/0", "no ip access-group HITS in"]));
    let acls = acls_of(&e);
    let lines = del_list_lines("HITS", &acls, &[], &|_: u16| s(""));
    t.eq("a named list: its line", lines.clone(), Ok(strs(&["no ip access-list extended HITS"])));
    t.eq("a named list goes", conf(&mut e, &lines.unwrap_or_default()), s(""));
    t.check("a named list is gone", !acls_of(&e).lists.contains_key("HITS"));
    let _ = conf(&mut e, &strs(&["access-list 20 permit any"]));
    let acls = acls_of(&e);
    let lines = del_list_lines("20", &acls, &[], &|_: u16| s(""));
    t.eq("a numbered list: its line", lines.clone(), Ok(strs(&["no access-list 20"])));
    t.eq("a numbered list goes", conf(&mut e, &lines.unwrap_or_default()), s(""));
    t.check("a numbered list is gone", !acls_of(&e).lists.contains_key("20"));

    // remarks stay with their rules, none gets lost; a raw line stays as it is
    let mut e = engine();
    let _ = conf(&mut e, &strs(&["access-list 150 remark zu A", "access-list 150 permit tcp any any eq 1", "access-list 150 permit 112 any any", "access-list 150 remark zu C", "access-list 150 permit tcp any any eq 3", "access-list 150 remark am Ende"]));
    let (ra, a_, b_, rc, c_, end) = (s("remark zu A"), s("permit tcp any any eq 1"), s("permit 112 any any"), s("remark zu C"), s("permit tcp any any eq 3"), s("remark am Ende"));
    let step = |e: &mut Engine, seq: u32, up: bool, label: &str, want: Vec<String>, t: &mut T| {
        let a = list_of(e, "150");
        let lines = move_lines("150", &a, seq, up).unwrap_or_default();
        t.eq(&format!("{}: the console takes them", label), conf(e, &lines), s(""));
        t.eq(&format!("{}: the order", label), texts(&list_of(e, "150")), want);
        lines
    };
    // the raw line goes up over A and its remark: by number, the remark stays above A
    let lines = step(&mut e, 20, true, "raw up", vec![b_.clone(), ra.clone(), a_.clone(), rc.clone(), c_.clone(), end.clone()], t);
    t.eq("raw up: by number", lines, strs(&["ip access-list extended 150", "no 20", "5 permit 112 any any"]));
    // C with its remark up over A with its: both must go and come again
    let c_seq = list_of(&e, "150").lines.iter().find(|l| l.item == Item::Rule(wild_rule(3))).map_or(0, |l| l.seq);
    let lines = step(&mut e, c_seq, true, "with remarks", vec![b_.clone(), rc.clone(), c_.clone(), ra.clone(), a_.clone(), end.clone()], t);
    t.check("with remarks: by their words", lines.iter().any(|l| l == "no remark zu A") && lines.iter().any(|l| l == "remark zu A") && lines.iter().any(|l| l == "no permit tcp any any eq 3"));
    // a place down over the raw line for a rule without a remark: the neighbour goes up by number
    let a = list_of(&e, "150");
    let first = a.lines.iter().find(|l| matches!(l.item, Item::Rule(_))).map_or(0, |l| l.seq);
    let _ = step(&mut e, first, false, "raw down", vec![rc.clone(), c_.clone(), b_.clone(), ra.clone(), a_.clone(), end.clone()], t);

    // adding, changing, deleting: what stays
    let mut e = engine();
    let _ = conf(&mut e, &strs(&["access-list 160 permit tcp any any eq 1", "access-list 160 permit tcp any any eq 2", "access-list 160 remark zu 3", "access-list 160 permit tcp any any eq 3"]));
    let a = list_of(&e, "160");
    let lines = add_lines("160", &a, &wild_rule(8), "neu", None).unwrap_or_default();
    t.eq("add at the end", lines.clone(), strs(&["access-list 160 remark neu", "access-list 160 permit tcp any any eq 8"]));
    t.eq("add at the end: taken", conf(&mut e, &lines), s(""));
    let a = list_of(&e, "160");
    let lines = add_lines("160", &a, &wild_rule(5), "", Some(10)).unwrap_or_default();
    t.eq("add after a rule: by number", lines.clone(), strs(&["ip access-list extended 160", "15 permit tcp any any eq 5"]));
    t.eq("add after a rule: taken", conf(&mut e, &lines), s(""));
    let want = strs(&["permit tcp any any eq 1", "permit tcp any any eq 5", "permit tcp any any eq 2", "remark zu 3", "permit tcp any any eq 3", "remark neu", "permit tcp any any eq 8"]);
    t.eq("add after a rule: the order", texts(&list_of(&e, "160")), want);
    // a new rule with a remark in the middle: what follows comes again
    let a = list_of(&e, "160");
    let lines = add_lines("160", &a, &wild_rule(6), "zu 6", Some(15)).unwrap_or_default();
    t.eq("a remark into the middle: taken", conf(&mut e, &lines), s(""));
    let want = strs(&["permit tcp any any eq 1", "permit tcp any any eq 5", "remark zu 6", "permit tcp any any eq 6", "permit tcp any any eq 2", "remark zu 3", "permit tcp any any eq 3", "remark neu", "permit tcp any any eq 8"]);
    t.eq("a remark into the middle: the order", texts(&list_of(&e, "160")), want);
    // a change keeps its place
    let a = list_of(&e, "160");
    let seq = a.lines.iter().find(|l| l.item == Item::Rule(wild_rule(2))).map_or(0, |l| l.seq);
    let lines = edit_lines("160", &a, seq, &wild_rule(22), "").unwrap_or_default();
    t.eq("a change: taken", conf(&mut e, &lines), s(""));
    let want = strs(&["permit tcp any any eq 1", "permit tcp any any eq 5", "remark zu 6", "permit tcp any any eq 6", "permit tcp any any eq 22", "remark zu 3", "permit tcp any any eq 3", "remark neu", "permit tcp any any eq 8"]);
    t.eq("a change: the order", texts(&list_of(&e, "160")), want);
    // of a rule with a remark above it, and of the remark
    let a = list_of(&e, "160");
    let seq = a.lines.iter().find(|l| l.item == Item::Rule(wild_rule(3))).map_or(0, |l| l.seq);
    let lines = edit_lines("160", &a, seq, &wild_rule(33), "zu 33").unwrap_or_default();
    t.eq("a change with its remark: taken", conf(&mut e, &lines), s(""));
    let want = strs(&["permit tcp any any eq 1", "permit tcp any any eq 5", "remark zu 6", "permit tcp any any eq 6", "permit tcp any any eq 22", "remark zu 33", "permit tcp any any eq 33", "remark neu", "permit tcp any any eq 8"]);
    t.eq("a change with its remark: the order", texts(&list_of(&e, "160")), want);
    // deleting takes the remarks above the rule, nothing else
    let a = list_of(&e, "160");
    let seq = a.lines.iter().find(|l| l.item == Item::Rule(wild_rule(33))).map_or(0, |l| l.seq);
    let lines = delete_lines("160", &a, seq as u64);
    t.eq("delete: by its words", lines.clone(), strs(&["ip access-list extended 160", "no remark zu 33", "no permit tcp any any eq 33"]));
    t.eq("delete: taken", conf(&mut e, &lines), s(""));
    let want = strs(&["permit tcp any any eq 1", "permit tcp any any eq 5", "remark zu 6", "permit tcp any any eq 6", "permit tcp any any eq 22", "remark neu", "permit tcp any any eq 8"]);
    t.eq("delete: the order", texts(&list_of(&e, "160")), want);
    // a remark after the last rule has a row of its own
    let _ = conf(&mut e, &strs(&["access-list 160 remark ganz unten"]));
    let a = list_of(&e, "160");
    let key = rows_of(&a).iter().find(|r| r.remark == "ganz unten").map_or(0, |r| r.key);
    let lines = delete_lines("160", &a, key);
    t.eq("delete a remark", lines.clone(), strs(&["ip access-list extended 160", "no remark ganz unten"]));
    t.eq("delete a remark: taken", conf(&mut e, &lines), s(""));
    // binding
    t.eq("bind", bind_lines("Gi0/0/0", false, None, Some("110")), strs(&["interface Gi0/0/0", "ip access-group 110 in"]));
    t.eq("bind another", bind_lines("Gi0/0/0", true, Some("110"), Some("WEB")), strs(&["interface Gi0/0/0", "ip access-group WEB out"]));
    t.eq("loosen", bind_lines("Gi0/0/0", true, Some("WEB"), None), strs(&["interface Gi0/0/0", "no ip access-group WEB out"]));
    t.eq("nothing bound, nothing to do", bind_lines("Gi0/0/0", true, None, None), Vec::<String>::new());
    let mut e = engine();
    t.eq("the console binds", conf(&mut e, &bind_lines("GigabitEthernet0/0/1", true, None, Some("160"))), s(""));
    t.eq("bound", acls_of(&e).bound.get(&(1, true)).cloned(), Some(s("160")));
    t.eq("the console loosens", conf(&mut e, &bind_lines("GigabitEthernet0/0/1", true, Some("160"), None)), s(""));
    t.eq("loose", acls_of(&e).bound.len(), 0);
}

// "permit tcp any any eq N"
fn wild_rule(port: u16) -> Ace {
    Ace { permit: true, proto: Some(PROTO_TCP), src: Wild::ANY, sport: Ports::Any, dst: Wild::ANY, dport: Ports::Eq(port), established: false, icmp: None, log: false }
}

// the pure question to the lists
fn probing(t: &mut T) {
    let f = Flow { proto: PROTO_TCP, src: ip("10.0.0.1"), dst: ip("10.0.0.2"), sport: 1000, dport: 80, ports: true, icmp: None, flags: 0, later: false, mf: false, id: 0 };
    let mut acls = Acls::default();
    t.eq("no list", acls.probe(0, false, &f), Probe::NoList);
    acls.bound.insert((0, false), s("X"));
    t.eq("a list that does not exist", acls.probe(0, false, &f), Probe::Empty(s("X")));
    acls.lists.insert(s("X"), Acl::new(AclKind::Extended, false));
    t.eq("a list without rules", acls.probe(0, false, &f), Probe::Empty(s("X")));
    if let Some(a) = acls.lists.get_mut("X") {
        let _ = a.add(None, Item::Remark(s("nur eine Bemerkung")));
    }
    t.eq("a list of remarks", acls.probe(0, false, &f), Probe::Empty(s("X")));
    t.eq("the other direction", acls.probe(0, true, &f), Probe::NoList);
    t.eq("the other interface", acls.probe(1, false, &f), Probe::NoList);
    if let Some(a) = acls.lists.get_mut("X") {
        let _ = a.add(None, Item::Rule(wild_rule(443)));
        let _ = a.add(None, Item::Rule(wild_rule(80)));
    }
    t.eq("a rule decides", acls.probe(0, false, &f), Probe::Decided { list: s("X"), verdict: Verdict { permit: true, seq: 20 }, line: Some(2) });
    t.eq("nothing counted", acls.lists.get("X").map(|a| a.lines.iter().map(|l| l.hits).sum::<u64>()), Some(0));
    let g = Flow { dport: 22, ..f };
    t.eq("the implicit deny", acls.probe(0, false, &g), Probe::Decided { list: s("X"), verdict: Verdict { permit: false, seq: 0 }, line: None });
    // what the part says
    let a = told(&acls, 0, "Gi0/0/0", false, &f);
    t.eq("told: a rule", (a.pass, a.seq, a.text), (true, 20, s("Liste X, Zeile 20: permit tcp any any eq www")));
    let a = told(&acls, 0, "Gi0/0/0", false, &g);
    t.check("told: the implicit deny", !a.pass && a.seq == 0 && a.text.contains("implizite deny"));
    let a = told(&acls, 0, "Gi0/0/0", true, &f);
    t.check("told: no list", a.pass && a.list.is_empty() && a.text.contains("Gi0/0/0") && a.text.contains("ausgehend"));
    acls.bound.insert((1, false), s("Y"));
    t.check("told: no such list", told(&acls, 1, "Gi0/0/1", false, &f).text.contains("gibt es nicht"));
    acls.lists.insert(s("Y"), Acl::new(AclKind::Extended, false));
    t.check("told: no rules", told(&acls, 1, "Gi0/0/1", false, &f).text.contains("keine Regeln"));
    // the packet of the fields
    let mut x = TestForm::new();
    x.src = s("10.0.0.1");
    x.dst = s("10.0.0.2");
    t.check("flow: a port is needed", flow_of(&x).is_err());
    x.dport = s("www");
    let fl = flow_of(&x).ok();
    t.eq("flow: tcp", fl.map(|f| (f.proto, f.dport, f.sport, f.ports, f.flags)), Some((PROTO_TCP, 80, 49152, true, netpkt::tcp::SYN)));
    x.ack = 1;
    x.sport = s("1234");
    t.eq("flow: a segment with ACK", flow_of(&x).ok().map(|f| (f.sport, f.flags)), Some((1234, netpkt::tcp::ACK)));
    x.src = s("hugo");
    t.check("flow: no address", flow_of(&x).err().is_some_and(|e| e.contains("hugo")));
    x.src = s("10.0.0.1");
    x.proto = 2;
    t.eq("flow: an echo request", flow_of(&x).ok().map(|f| (f.proto, f.icmp, f.ports)), Some((PROTO_ICMP, Some((8, 0)), true)));
    x.icmp = jfn_sim::acl::ICMP_TYPES.iter().position(|m| m.0 == "port-unreachable").unwrap_or(0) as i32;
    t.eq("flow: an ICMP error", flow_of(&x).ok().map(|f| (f.icmp, f.ports)), Some((Some((3, 3)), false)));
    x.proto = test_protos().iter().position(|p| p.0 == "gre").unwrap_or(0) as i32;
    t.eq("flow: another protocol", flow_of(&x).ok().map(|f| (f.proto, f.ports)), Some((47, false)));
}

// NAT: what the forms type, on the engine
fn nat_forms(t: &mut T) {
    let mut e = engine();
    let g0 = "GigabitEthernet0/0/0";
    let g1 = "GigabitEthernet0/0/1";
    t.eq("side inside", side_lines(g0, None, Some(Side::Inside)), strs(&["interface GigabitEthernet0/0/0", "ip nat inside"]));
    t.eq("side outside", side_lines(g1, Some(Side::Inside), Some(Side::Outside)), strs(&["interface GigabitEthernet0/0/1", "ip nat outside"]));
    t.eq("no side", side_lines(g1, Some(Side::Outside), None), strs(&["interface GigabitEthernet0/0/1", "no ip nat outside"]));
    t.eq("no side, nothing to do", side_lines(g1, None, None), Vec::<String>::new());
    t.eq("the console takes the sides", conf(&mut e, &side_lines(g0, None, Some(Side::Inside))) + &conf(&mut e, &side_lines(g1, None, Some(Side::Outside))), s(""));
    let nat = |e: &Engine| e.nat(R1).cloned().unwrap_or_default();
    t.eq("sides", (nat(&e).side(0), nat(&e).side(1)), (Some(Side::Inside), Some(Side::Outside)));
    // pools
    let want = Ok(strs(&["ip nat pool P 203.0.113.100 203.0.113.110 netmask 255.255.255.0"]));
    for m in ["24", "/24", "255.255.255.0"] {
        t.eq(&format!("pool with {}", m), pool_lines(" P ", " 203.0.113.100", "203.0.113.110 ", m), want.clone());
    }
    for (name, a, b, m, word) in [("", "1.1.1.1", "1.1.1.2", "24", "Pool"), ("a b", "1.1.1.1", "1.1.1.2", "24", "Pool"), ("P", "1.1.1", "1.1.1.2", "24", "IPv4"), ("P", "1.1.1.1", "x", "24", "IPv4"), ("P", "1.1.1.1", "1.1.1.2", "33", "Netzmaske"), ("P", "1.1.1.1", "1.1.1.2", "255.0.255.0", "Netzmaske"), ("P", "1.1.1.1", "1.1.2.1", "24", "/24"), ("P", "1.1.1.9", "1.1.1.2", "24", "/24")] {
        t.check(&format!("pool refused: {} {} {} {}", name, a, b, m), pool_lines(name, a, b, m).err().is_some_and(|x| x.contains(word)));
    }
    t.eq("the console takes the pool", conf(&mut e, &want.clone().unwrap_or_default()), s(""));
    t.check("the pool is there", nat(&e).pools.contains_key("P"));
    // dynamic rules
    let _ = conf(&mut e, &strs(&["access-list 1 permit 192.168.1.0 0.0.0.255"]));
    let to_if = Target::Iface(s(g1));
    let to_pool = Target::Pool(s("P"));
    t.eq("PAT behind an interface", dyn_lines("1", &to_if, true), strs(&["ip nat inside source list 1 interface GigabitEthernet0/0/1 overload"]));
    t.eq("a pool", dyn_lines("1", &to_pool, false), strs(&["ip nat inside source list 1 pool P"]));
    t.eq("a pool with PAT", dyn_lines("1", &to_pool, true), strs(&["ip nat inside source list 1 pool P overload"]));
    t.eq("the console takes it", conf(&mut e, &dyn_lines("1", &to_if, true)), s(""));
    t.eq("a rule", nat(&e).rules.len(), 1);
    let r = nat(&e).rules[0].1.clone();
    t.eq("and the line that takes it away", rule_del_line(&r), s("no ip nat inside source list 1"));
    t.eq("the console takes that", conf(&mut e, &[rule_del_line(&r)]), s(""));
    t.eq("no rule", nat(&e).rules.len(), 0);
    t.eq("a pool in use: the console takes the rule", conf(&mut e, &dyn_lines("1", &to_pool, true)), s(""));
    let rules: Vec<Rule> = nat(&e).rules.iter().map(|x| x.1.clone()).collect();
    t.check("the form keeps a pool a rule reads", pool_del_lines("P", &rules).err().is_some_and(|x| x.contains("Liste 1") && x.contains("Pool P")));
    let _ = conf(&mut e, &[rule_del_line(&rules[0])]);
    t.eq("without the rule it can go", pool_del_lines("P", &[]), Ok(strs(&["no ip nat pool P"])));
    t.eq("the console takes that", conf(&mut e, &strs(&["no ip nat pool P"])), s(""));
    t.check("no pool", nat(&e).pools.is_empty());
    // static rules and port forwarding
    let ifs = strs(&[g0, g1]);
    let form = |kind, local: &str, lport: &str, gtype, global: &str, gif, gport: &str| StaticForm { kind, local: s(local), lport: s(lport), gtype, global: s(global), gif, gport: s(gport) };
    let cases = [
        (form(0, "192.168.1.30", "", 0, "203.0.113.30", 0, ""), "ip nat inside source static 192.168.1.30 203.0.113.30", "no ip nat inside source static 192.168.1.30"),
        (form(0, "192.168.1.31", "", 1, "", 1, ""), "ip nat inside source static 192.168.1.31 interface GigabitEthernet0/0/1", "no ip nat inside source static 192.168.1.31"),
        (form(1, "192.168.1.20", "80", 1, "", 1, "8080"), "ip nat inside source static tcp 192.168.1.20 80 interface GigabitEthernet0/0/1 8080", "no ip nat inside source static tcp 192.168.1.20 80"),
        (form(2, "192.168.1.20", "53", 0, "203.0.113.5", 0, "5353"), "ip nat inside source static udp 192.168.1.20 53 203.0.113.5 5353", "no ip nat inside source static udp 192.168.1.20 53"),
    ];
    for (f, line, del) in &cases {
        t.eq(&format!("static: {}", line), static_lines(f, &ifs), Ok(strs(&[line])));
        t.eq(&format!("the console takes: {}", line), conf(&mut e, &strs(&[line])), s(""));
        let last = nat(&e).rules.last().map(|x| x.1.clone());
        t.eq(&format!("the line that removes: {}", line), last.as_ref().map(rule_del_line), Some(s(del)));
        t.check(&format!("the configuration has: {}", line), e.running_config(R1).contains(line));
    }
    let n0 = nat(&e).rules.len();
    t.eq("all four", n0, 4);
    for (_, _, del) in &cases {
        t.eq(&format!("the console removes: {}", del), conf(&mut e, &strs(&[del])), s(""));
    }
    t.eq("all gone", nat(&e).rules.len(), 0);
    for (f, word) in [(form(0, "x", "", 0, "1.1.1.1", 0, ""), "IPv4"), (form(0, "1.1.1.1", "", 0, "y", 0, ""), "IPv4"), (form(1, "1.1.1.1", "0", 0, "2.2.2.2", 0, "80"), "Port"), (form(1, "1.1.1.1", "80", 0, "2.2.2.2", 0, "70000"), "Port"), (form(1, "1.1.1.1", "", 0, "2.2.2.2", 0, "80"), "Port"), (form(0, "1.1.1.1", "", 1, "", 7, ""), "Schnittstelle")] {
        t.check(&format!("static refused: {:?}", f), static_lines(&f, &ifs).err().is_some_and(|x| x.contains(word)));
    }
    // the translations and their clear button
    let _ = conf(&mut e, &strs(&["ip nat inside source list 1 interface GigabitEthernet0/0/1 overload"]));
    warm(&mut e);
    t.check("translations run", !e.nat_translations(R1).is_empty());
    t.eq("the clear button's line", conf(&mut e, &strs(&["do clear ip nat translation *"])), s(""));
    t.eq("no translation left", e.nat_translations(R1).len(), 0);
}

/// the packets of the table: who sends what
#[derive(Clone, Copy)]
enum Send {
    Tcp(u16),
    Udp(u16),
    Ping,
}

// the first filter note about a packet since `from` events: (permit, seq)
fn first_verdict(e: &Engine, from: usize, iface: u16, out: bool) -> Option<(bool, u32, String)> {
    e.trace.events().skip(from).filter(|x| x.dev == R1).find_map(|x| match &x.note {
        Note::AclPermit { list, seq, iface: i, out: o } if *i == iface && *o == out => Some((true, *seq, list.clone())),
        Note::AclDeny { list, seq, iface: i, out: o } if *i == iface && *o == out => Some((false, *seq, list.clone())),
        _ => None,
    })
}

// one packet: the fields, the prediction, the packet for real
fn check_packet(t: &mut T, e: &mut Engine, name: &str, from: (u32, &str), src_seen: &str, what: Send, iface: u16, out_: bool) {
    let mut f = TestForm::new();
    f.src = s(src_seen);
    f.dst = s("203.0.113.10");
    let (proto, dport) = match what {
        Send::Tcp(p) => (PROTO_TCP, p),
        Send::Udp(p) => (PROTO_UDP, p),
        Send::Ping => (PROTO_ICMP, 0),
    };
    // (a source port of its own for each: a connection before this one may linger)
    let sport = 40000 + dport % 20000;
    f.proto = test_protos().iter().position(|p| p.1 == proto).unwrap_or(0) as i32;
    if proto != PROTO_ICMP {
        f.sport = format!("{}", sport);
        f.dport = format!("{}", dport);
    }
    let Ok(flow) = flow_of(&f) else {
        t.check(&format!("{}: the fields", name), false);
        return;
    };
    let acls = acls_of(e);
    let told_now = told(&acls, iface, "Gi0/0/x", out_, &flow);
    let hits_before: u64 = acls.lists.values().flat_map(|a| a.lines.iter()).map(|l| l.hits).sum();
    let from_ev = e.trace.events().count();
    match what {
        Send::Tcp(p) => {
            let _ = pdu(e, from.0, PduProto::Tcp, "203.0.113.10", sport, p);
        }
        Send::Udp(p) => {
            let _ = pdu(e, from.0, PduProto::Udp, "203.0.113.10", sport, p);
        }
        Send::Ping => {
            let _ = e.ping(from.0, ip("203.0.113.10"), PingOpts { count: 1, ..PingOpts::default() });
        }
    }
    // (asking changed nothing)
    t.eq(&format!("{}: asking counts nothing", name), acls_of(e).lists.values().flat_map(|a| a.lines.iter()).map(|l| l.hits).sum::<u64>(), hits_before);
    run_for(e, 50 * MS);
    let real = first_verdict(e, from_ev, iface, out_);
    t.eq(&format!("{}: the engine's verdict is the prediction", name), real, Some((told_now.pass, told_now.seq, told_now.list.clone())));
    if told_now.seq != 0 {
        let hits_now: u64 = acls_of(e).lists.values().flat_map(|a| a.lines.iter()).map(|l| l.hits).sum();
        t.check(&format!("{}: the rule counted it", name), hits_now > hits_before);
    }
}

// the table: the prediction against packets sent for real
fn packets(t: &mut T) {
    // inbound on Gi0/0/0
    let mut e = engine();
    warm(&mut e);
    let o = conf(&mut e, &strs(&[
        "ip access-list extended IN",
        "deny tcp host 192.168.1.11 any eq www",
        "permit tcp any any eq www",
        "permit udp any any eq domain",
        "deny udp any any range 1000 2000",
        "permit udp any gt 39999 any eq 7777",
        "deny udp any lt 40000 any eq 7778",
        "permit icmp any any echo",
        "exit",
        "interface GigabitEthernet0/0/0",
        "ip access-group IN in",
    ]));
    t.eq("the inbound list", o, s(""));
    let table: [(&str, (u32, &str), Send); 10] = [
        ("PC1 tcp 80: the second rule", (PC1, "192.168.1.10"), Send::Tcp(80)),
        ("PC2 tcp 80: the first rule", (PC2, "192.168.1.11"), Send::Tcp(80)),
        ("PC1 tcp 22: implicit deny", (PC1, "192.168.1.10"), Send::Tcp(22)),
        ("PC1 udp 53", (PC1, "192.168.1.10"), Send::Udp(53)),
        ("PC2 udp 1500", (PC2, "192.168.1.11"), Send::Udp(1500)),
        ("PC2 udp 2001: implicit deny", (PC2, "192.168.1.11"), Send::Udp(2001)),
        ("PC1 udp 7777 by the source port", (PC1, "192.168.1.10"), Send::Udp(7777)),
        ("PC1 udp 7778 past the deny of low ports", (PC1, "192.168.1.10"), Send::Udp(7778)),
        ("PC1 ping", (PC1, "192.168.1.10"), Send::Ping),
        ("PC2 ping", (PC2, "192.168.1.11"), Send::Ping),
    ];
    for (name, from, what) in table {
        check_packet(t, &mut e, name, from, from.1, what, 0, false);
    }
    // outbound on Gi0/0/1
    let mut e = engine();
    warm(&mut e);
    let o = conf(&mut e, &strs(&["access-list 130 deny tcp any any eq telnet", "access-list 130 permit tcp 192.168.1.0 0.0.0.255 any", "access-list 130 deny icmp host 192.168.1.11 any", "access-list 130 permit icmp any any", "interface GigabitEthernet0/0/1", "ip access-group 130 out"]));
    t.eq("the outbound list", o, s(""));
    for (name, from, what) in [("out: telnet", (PC1, "192.168.1.10"), Send::Tcp(23)), ("out: web", (PC1, "192.168.1.10"), Send::Tcp(80)), ("out: udp, implicit deny", (PC1, "192.168.1.10"), Send::Udp(53)), ("out: PC2's ping", (PC2, "192.168.1.11"), Send::Ping), ("out: PC1's ping", (PC1, "192.168.1.10"), Send::Ping)] {
        check_packet(t, &mut e, name, from, from.1, what, 1, true);
    }
    // outbound after NAT: the list sees the global source, and so does the question
    let mut e = engine();
    let o = conf(&mut e, &strs(&["access-list 1 permit 192.168.1.0 0.0.0.255", "interface GigabitEthernet0/0/0", "ip nat inside", "interface GigabitEthernet0/0/1", "ip nat outside", "exit", "ip nat inside source list 1 interface GigabitEthernet0/0/1 overload"]));
    t.eq("NAT", o, s(""));
    warm(&mut e);
    let o = conf(&mut e, &strs(&["access-list 140 deny udp host 192.168.1.10 any", "access-list 140 permit udp host 203.0.113.1 any eq 9999", "access-list 140 permit icmp any any", "interface GigabitEthernet0/0/1", "ip access-group 140 out"]));
    t.eq("the list behind NAT", o, s(""));
    check_packet(t, &mut e, "after NAT: the global source meets the rule", (PC1, ""), "203.0.113.1", Send::Udp(9999), 1, true);
    check_packet(t, &mut e, "after NAT: the global source, implicit deny", (PC1, ""), "203.0.113.1", Send::Udp(9998), 1, true);
    // an unbound and an empty list let it through: the engine notes nothing
    let mut e = engine();
    warm(&mut e);
    let _ = conf(&mut e, &strs(&["ip access-list extended LEER", "exit", "interface GigabitEthernet0/0/0", "ip access-group LEER in"]));
    let acls = acls_of(&e);
    let mut f = TestForm::new();
    f.src = s("192.168.1.10");
    f.dst = s("203.0.113.10");
    f.dport = s("80");
    let told_now = flow_of(&f).map(|fl| told(&acls, 0, "Gi0/0/0", false, &fl));
    t.check("empty: it passes", told_now.is_ok_and(|x| x.pass && x.text.contains("keine Regeln")));
    let from_ev = e.trace.events().count();
    let name = pdu(&mut e, PC1, PduProto::Tcp, "203.0.113.10", 40000, 80);
    run_for(&mut e, 50 * MS);
    t.eq("empty: the engine does not filter", first_verdict(&e, from_ev, 0, false), None);
    t.eq("empty: nothing failed", e.service::<PduSender>(PC1, &name).map(|x| x.failed), Some(0));
}

// the steps of the window: console::run_lines_running on a Net
fn steps(t: &mut T) {
    let mut n = crate::app::Net::new();
    n.doc.replace(project(), None);
    n.doc.sim.start();
    let list = |n: &crate::app::Net, name: &str| n.doc.sim.state().node(R1).and_then(|x| x.ip.as_ref()).and_then(|st| st.acls.lists.get(name).cloned());
    let startup = |n: &crate::app::Net| n.doc.proj.device(R1).map(|d| d.startup_config.clone()).unwrap_or_default();
    let first = startup(&n);
    // a list typed on the console and not saved: it shows, and the project does not have it
    for l in ["enable", "conf t", "access-list 150 permit tcp any any eq 1", "access-list 150 permit tcp any any eq 2", "access-list 150 permit tcp any any eq 3", "end"] {
        console::typed(&mut n, R1, l);
    }
    t.eq("typed: it shows", list(&n, "150").map(|a| texts(&a).len()), Some(3));
    t.eq("typed: not in the project", startup(&n), first.clone());
    // the third rule up, one step: running and startup agree, what was typed included
    let a = list(&n, "150").unwrap_or_else(|| Acl::new(AclKind::Extended, true));
    let lines = move_lines("150", &a, 30, true).unwrap_or_default();
    t.eq("moved", console::run_lines_running(&mut n, R1, "verschieben", &lines), Ok(()));
    let want = strs(&["permit tcp any any eq 1", "permit tcp any any eq 3", "permit tcp any any eq 2"]);
    t.eq("moved: running", list(&n, "150").map(|a| texts(&a)), Some(want.clone()));
    t.check("moved: startup in the same order", startup(&n).contains("access-list 150 permit tcp any any eq 1\naccess-list 150 permit tcp any any eq 3\naccess-list 150 permit tcp any any eq 2\n"));
    t.eq("moved: the numbers", list(&n, "150").map(|a| a.lines.iter().map(|l| l.seq).collect::<Vec<u32>>()), Some(vec![10, 15, 20]));
    // what the console shows of it
    let shown_cons = n.cons.get(&n.doc.proj, R1).term.text();
    t.check("moved: the lines are on the console", shown_cons.contains("no 30") && shown_cons.contains("15 permit tcp any any eq 3"));
    // numbers not 10, 20, ... in the running list: the scratch copy still agrees
    let a = list(&n, "150").unwrap_or_else(|| Acl::new(AclKind::Extended, true));
    let lines = move_lines("150", &a, 20, true).unwrap_or_default();
    t.eq("again", console::run_lines_running(&mut n, R1, "verschieben", &lines), Ok(()));
    t.check("again: startup in the same order", startup(&n).contains("access-list 150 permit tcp any any eq 1\naccess-list 150 permit tcp any any eq 2\naccess-list 150 permit tcp any any eq 3\n"));
    // a refusal changes nothing
    let before = startup(&n);
    t.check("a refusal says why", console::run_lines_running(&mut n, R1, "x", &strs(&["no ip nat pool GIBTSNICHT"])).err().is_some_and(|x| x.contains("R1") && x.contains("does not exist")));
    t.eq("a refusal changes nothing", startup(&n), before);
    // one undo step each; undo restarts with the old configuration
    t.check("undo", n.doc.undo());
    t.check("undo: the step before", startup(&n).contains("access-list 150 permit tcp any any eq 1\naccess-list 150 permit tcp any any eq 3\n"));
    t.check("undo again", n.doc.undo());
    t.eq("undo again: the project as it was", startup(&n), first);
    t.check("undo again: what was only typed is gone with the restart", list(&n, "150").is_none());
    t.check("nothing more to undo", !n.doc.undo());
    // run_lines itself still starts from the project
    for l in ["", "enable", "conf t", "access-list 7 permit any", "end"] {
        console::typed(&mut n, R1, l);
    }
    t.eq("run_lines", console::run_lines(&mut n, R1, "x", &strs(&["ip route 10.9.0.0 255.255.0.0 203.0.113.10"])), Ok(()));
    t.check("run_lines: the typed list is not saved", !startup(&n).contains("access-list 7"));
    t.check("run_lines: the line is", startup(&n).contains("ip route 10.9.0.0"));
    // the tab: on IOS-like devices that route
    let mut p = Project::new("tabs", 1);
    let models = [("jf-fw", true), ("jf-fw2", true), ("jf-fw8", true), ("jf-r4", true), ("jf-s24l3", true), ("jf-s24p", false), ("pc", false), ("home-router", false), ("hub", false)];
    let ids: Vec<u32> = models.iter().map(|(m, _)| p.add_device(m, 0, 0).unwrap_or(0)).collect();
    let mut w = crate::app::Net::new();
    w.doc.replace(p, None);
    for (id, (m, want)) in ids.iter().zip(models) {
        t.eq(&format!("the tab on {}", m), crate::workspace::devwin::tabs_of(&w, *id).contains(&crate::workspace::devwin::Tab::Firewall), want);
    }
    // a button that changes no configuration is no undo step
    let mut n = crate::app::Net::new();
    n.doc.replace(project(), None);
    n.doc.sim.start();
    t.eq("clear", console::run_lines_running(&mut n, R1, "x", &strs(&["do clear ip nat translation *"])), Ok(()));
    t.check("clear: no step", !n.doc.undo());
    // without routing: an L3 switch before "ip routing"
    let mut p = Project::new("l3", 1);
    let sw = p.add_device("jf-s24l3", 0, 0).unwrap_or(0);
    let m = p.model_of(sw).map(|m| m.has(jfn_model::catalog::F_ROUTER));
    let e = Engine::from_project(&p).unwrap_or_else(|_| Engine::new(0));
    t.eq("an L3 switch routes after ip routing", (m, e.node(sw).and_then(|x| x.ip.as_ref()).map(|st| st.forwarding)), (Some(true), Some(false)));
}
