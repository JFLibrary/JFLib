// riptest -- checks of RIP in jfn_sim (group rip of --selftest; jfn_sim::rip, cli/rip.rs): the
// first exchange with exact virtual times, convergence and metrics (RFC 2453: a connected network
// costs 1, every hop adds 1), periodic updates with their random offset and triggered ones with
// their hold, split horizon with poisoned reverse on the wire, a cable pulled (triggered poison,
// holddown, garbage collection), a silent neighbour (timeout and flush), passive interfaces,
// distances against static routes, equal-cost paths, version 1 next to version 2 (broadcasts,
// classful masks, summaries), auto-summary, the default route and redistribution, RIP on an L3
// switch's VLAN interface, the command line (modes, help, show, debug, clear, the running
// configuration read back), power, the notes in German and English, determinism
//
// Times: 1 Gbit/s copper with 50 ns of cable, so a frame of n bytes takes (max(n, 60) + 24) * 8
// ns on the wire plus 50: a request (66 bytes) 770 ns, an update with k routes (46 + 20 k bytes)
// (70 + 20 k) * 8 + 50 ns; frames out of one port follow each other.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use jfn_model::project::{PortRef, Project};
use jfn_sim::{Action, Engine, Note, PingOpts, Report, RipDrop, RipLoss, RipOut, RouteSrc, MS, SEC};
use netpkt::decode;
use netpkt::rip::{self, RipView};
use netpkt::{Ipv4, Prefix4};

use crate::selftest::T;

const BUDGET: usize = 8_000_000;

fn pr(dev: u32, port: &str) -> PortRef {
    PortRef { dev, port: String::from(port) }
}

fn ip(s: &str) -> Ipv4 {
    Ipv4::parse(s).unwrap_or(Ipv4::UNSPECIFIED)
}

fn pf(s: &str) -> Prefix4 {
    Prefix4::parse(s).unwrap_or_default()
}

fn s(x: &str) -> String {
    String::from(x)
}

/// a startup configuration: host name, interfaces with an address each, then `tail`
fn cfg(name: &str, ifs: &[(&str, &str)], tail: &str) -> String {
    let mut c = format!("hostname {}\n!\n", name);
    for (i, a) in ifs {
        let p = pf(a);
        c.push_str(&format!("interface {}\n ip address {} {}\n no shutdown\n!\n", i, p.addr, p.mask()));
    }
    c.push_str(tail);
    c.push_str("end\n");
    c
}

/// the usual process: version 2 on 10.0.0.0
const RIP2: &str = "router rip\n version 2\n network 10.0.0.0\n no auto-summary\n!\n";

fn set(p: &mut Project, id: u32, text: String) {
    if let Some(d) = p.device_mut(id) {
        d.startup_config = text;
    }
}

fn wire(p: &mut Project, a: u32, ap: &str, b: u32, bp: &str) -> u32 {
    p.connect(&pr(a, ap), &pr(b, bp), None).unwrap_or(0)
}

fn engine(p: &Project) -> Engine {
    Engine::from_project(p).unwrap_or_else(|_| Engine::new(0))
}

fn run_to(e: &mut Engine, t: u64) {
    e.run_until(t, BUDGET);
}

/// what a device printed since the last call
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

/// lines typed at a device now (the engine does what is due at this moment); its output
fn cli(e: &mut Engine, dev: u32, lines: &[&str]) -> String {
    let _ = e.take_reports();
    for l in lines {
        let _ = e.cli_line(dev, l);
    }
    let now = e.now();
    e.run_until(now, BUDGET);
    out(e, dev)
}

/// a console refused a line ("% ..." at the start of a line; log lines start with their stamp)
fn refused(o: &str) -> bool {
    o.lines().any(|l| l.starts_with('%'))
}

/// the RIP decisions of a device with their times
fn rip_notes(e: &Engine, dev: u32) -> Vec<(u64, Note)> {
    e.trace.events().filter(|x| x.dev == dev && x.note.is_rip()).map(|x| (x.t, x.note.clone())).collect()
}

/// the first time a decision of a device matches
fn when(e: &Engine, dev: u32, f: impl Fn(&Note) -> bool) -> Option<u64> {
    e.trace.events().find(|x| x.dev == dev && f(&x.note)).map(|x| x.t)
}

/// the RIP routes of a device's RIB: (prefix, next hop, metric, distance)
fn rip_routes(e: &Engine, dev: u32) -> Vec<(String, Option<Ipv4>, u32, u8)> {
    e.routes(dev).into_iter().filter(|r| r.src == RouteSrc::Rip).map(|r| (format!("{}", r.prefix), r.via, r.metric, r.ad)).collect()
}

/// the routes of a prefix that win in the RIB (best distance): (source code, next hop)
fn best(e: &Engine, dev: u32, p: &str) -> Vec<(&'static str, Option<Ipv4>)> {
    let rs: Vec<_> = e.routes(dev).into_iter().filter(|r| r.prefix == pf(p)).collect();
    let ad = rs.iter().map(|r| r.ad).min();
    rs.into_iter().filter(|r| Some(r.ad) == ad).map(|r| (r.src.code(), r.via)).collect()
}

/// the RIP messages that crossed a cable, from `from`: (time sent, the message)
fn rip_frames(e: &Engine, link: u32, from: u32) -> Vec<(u64, rip::Rip)> {
    e.trace
        .hops()
        .filter(|h| h.link == link && h.from.0 == from && h.mask & decode::P_RIP != 0)
        .filter_map(|h| {
            let st = decode::stack(&h.bytes);
            let (o, l) = st.find(decode::Proto::Rip)?;
            RipView::parse(&h.bytes[o..o + l]).map(|v| (h.t_tx, rip::Rip::read(&v)))
        })
        .collect()
}

/// a message's routes as (prefix, metric)
fn entries(m: &rip::Rip) -> Vec<(String, u32)> {
    m.entries.iter().map(|x| (format!("{}", x.prefix().unwrap_or(Prefix4 { addr: x.addr, len: 32 })), x.metric)).collect()
}

/// R1 (Lo0 10.1.1.1/24) - Gi0/0/0 10.0.12.0/24 Gi0/0/0 - R2 (Lo0 10.2.2.2/24)
fn pair(tail1: &str, tail2: &str) -> (Project, u32, u32, u32) {
    let mut p = Project::new("rip pair", 31);
    let r1 = p.add_device("jf-r4", 0, 0).unwrap_or(0);
    let r2 = p.add_device("jf-r4", 200, 0).unwrap_or(0);
    let l = wire(&mut p, r1, "gi0/0/0", r2, "gi0/0/0");
    set(&mut p, r1, cfg("R1", &[("GigabitEthernet0/0/0", "10.0.12.1/24"), ("Loopback0", "10.1.1.1/24")], tail1));
    set(&mut p, r2, cfg("R2", &[("GigabitEthernet0/0/0", "10.0.12.2/24"), ("Loopback0", "10.2.2.2/24")], tail2));
    (p, r1, r2, l)
}

/// R1 (Lo0 10.1.1.1/24) - 10.0.12.0/24 - R2 - 10.0.23.0/24 - R3 (Lo0 10.3.3.3/24); the links
fn chain() -> (Project, [u32; 3], [u32; 2]) {
    let mut p = Project::new("rip chain", 32);
    let r: Vec<u32> = (0..3).map(|i| p.add_device("jf-r4", 200 * i, 0).unwrap_or(0)).collect();
    let l1 = wire(&mut p, r[0], "gi0/0/0", r[1], "gi0/0/0");
    let l2 = wire(&mut p, r[1], "gi0/0/1", r[2], "gi0/0/0");
    set(&mut p, r[0], cfg("R1", &[("GigabitEthernet0/0/0", "10.0.12.1/24"), ("Loopback0", "10.1.1.1/24")], RIP2));
    set(&mut p, r[1], cfg("R2", &[("GigabitEthernet0/0/0", "10.0.12.2/24"), ("GigabitEthernet0/0/1", "10.0.23.2/24")], RIP2));
    set(&mut p, r[2], cfg("R3", &[("GigabitEthernet0/0/0", "10.0.23.3/24"), ("Loopback0", "10.3.3.3/24")], RIP2));
    (p, [r[0], r[1], r[2]], [l1, l2])
}

pub fn run(t: &mut T) {
    first_exchange(t);
    convergence(t);
    split_horizon(t);
    cable_pulled(t);
    silent_neighbour(t);
    passive(t);
    distances(t);
    equal_cost(t);
    versions(t);
    summaries(t);
    default_and_redistribution(t);
    l3_switch(t);
    command_line(t);
    power(t);
    texts(t);
    determinism(t);
}

fn first_exchange(t: &mut T) {
    let (p, r1, r2, l) = pair(RIP2, RIP2);
    let mut e = engine(&p);
    t.eq("boot errors", (e.boot_errors(r1).len(), e.boot_errors(r2).len()), (0, 0));
    run_to(&mut e, 10 * MS);
    // at 0 each router names its own networks, asks for the table and sends its own (without the
    // network of the interface itself)
    let n0: Vec<Note> = rip_notes(&e, r1).into_iter().filter(|x| x.0 == 0).map(|x| x.1).collect();
    t.eq(
        "r1 at the start",
        n0,
        alloc::vec![
            Note::RipLocal { prefix: pf("10.0.12.0/24"), how: jfn_sim::RipHow::Connected },
            Note::RipLocal { prefix: pf("10.1.1.0/24"), how: jfn_sim::RipHow::Connected },
            Note::RipRequest { iface: 0, ver: 2 },
            Note::RipSent { iface: 0, ver: 2, to: rip::GROUP, kind: RipOut::Regular, routes: alloc::vec![(pf("10.1.1.0/24"), 1)], poisoned: 0 },
        ],
    );
    // R2's request (770 ns) and its update with one route right after it (720 + 720 + 50 ns)
    t.eq("request heard", when(&e, r1, |n| matches!(n, Note::RipAsked { whole: true, .. })), Some(770));
    t.eq("route learned", when(&e, r1, |n| *n == Note::RipLearned { prefix: pf("10.2.2.0/24"), via: ip("10.0.12.2"), metric: 2 }), Some(1490));
    t.eq("r1 rib", rip_routes(&e, r1), alloc::vec![(s("10.2.2.0/24"), Some(ip("10.0.12.2")), 2, 120)]);
    t.eq("r2 rib", rip_routes(&e, r2), alloc::vec![(s("10.1.1.0/24"), Some(ip("10.0.12.1")), 2, 120)]);
    // on the wire: a version 2 request for the whole table to 224.0.0.9 with TTL 1, then the update
    let h = e.trace.hops().find(|h| h.link == l && h.from.0 == r1 && h.mask & decode::P_RIP != 0).cloned();
    match h {
        Some(h) => {
            t.eq("first frame: to the group", (&h.bytes[0..6], h.t_tx), (&[0x01u8, 0x00, 0x5E, 0x00, 0x00, 0x09][..], 0));
            t.eq("ttl, ports", (h.bytes[22], &h.bytes[30..34], &h.bytes[34..38]), (1u8, &[224u8, 0, 0, 9][..], &[0x02u8, 0x08, 0x02, 0x08][..]));
            t.eq("summary", jfn_sim::view::hop_info(&h).1, s("RIPv2 Request for the whole table"));
        }
        None => t.check("first frame", false),
    }
    let msgs = rip_frames(&e, l, r1);
    t.check("request then update", msgs.len() >= 2 && msgs[0].1.is_request_all() && msgs[1].1.command == rip::CMD_RESPONSE && entries(&msgs[1].1) == alloc::vec![(s("10.1.1.0/24"), 1)]);
    // the answer to R2's request goes to R2 itself; the IOS-like ARP drops it while it asks
    t.check("answer unicast", rip_notes(&e, r1).iter().any(|x| matches!(&x.1, Note::RipSent { kind: RipOut::Answer, to, .. } if *to == ip("10.0.12.2"))));
    t.check("answer waits for arp", e.trace.events().any(|x| x.dev == r1 && x.note == Note::ArpIncomplete { ip: ip("10.0.12.2") }));
    // the service and its numbers
    let svcs = e.services(r1);
    t.check("service listed", svcs.iter().any(|x| x.kind == "rip" && x.name == "rip" && x.running));
    t.eq("metrics", (e.metric(r1, "rip", "routes"), e.metric(r1, "rip", "learned"), e.metric(r1, "rip", "neighbours")), (Some(3), Some(1), Some(1)));
    t.check("socket 520", e.sockets(r1).iter().any(|x| x.port == 520));
    t.check("group joined where rip runs", e.node(r1).and_then(|n| n.ip.as_ref()).is_some_and(|st| st.joined(0, rip::GROUP) && !st.joined(1, rip::GROUP)));
}

fn convergence(t: &mut T) {
    let (p, r, _) = chain();
    let mut e = engine(&p);
    run_to(&mut e, 40 * SEC);
    t.eq("r1 rib", rip_routes(&e, r[0]), alloc::vec![(s("10.0.23.0/24"), Some(ip("10.0.12.2")), 2, 120), (s("10.3.3.0/24"), Some(ip("10.0.12.2")), 3, 120)]);
    t.eq("r3 rib", rip_routes(&e, r[2]), alloc::vec![(s("10.0.12.0/24"), Some(ip("10.0.23.2")), 2, 120), (s("10.1.1.0/24"), Some(ip("10.0.23.2")), 3, 120)]);
    // R1 hears of 10.3.3.0/24 from R2's triggered update, one hop later than R2 did
    let r2_learned = when(&e, r[1], |n| matches!(n, Note::RipLearned { prefix, .. } if *prefix == pf("10.3.3.0/24")));
    let r1_learned = when(&e, r[0], |n| matches!(n, Note::RipLearned { prefix, .. } if *prefix == pf("10.3.3.0/24")));
    t.eq("learned in turn", (r2_learned, r1_learned), (Some(1490), Some(PROBE_CHAIN_R1)));
    // periodic updates: every 30 s plus or minus 5 s, from the device's own random numbers
    let regular: Vec<u64> = rip_notes(&e, r[0]).into_iter().filter(|x| matches!(x.1, Note::RipSent { kind: RipOut::Regular, .. })).map(|x| x.0).collect();
    t.eq("r1 updates", regular.clone(), PROBE_CHAIN_REGULAR.to_vec());
    t.check("periods within 25..35 s", regular.windows(2).filter(|w| w[1] != w[0]).all(|w| (25 * SEC..=35 * SEC).contains(&(w[1] - w[0]))));
    // triggered updates wait 1 to 5 s after one went out
    let trig: Vec<u64> = rip_notes(&e, r[1]).into_iter().filter(|x| matches!(x.1, Note::RipSent { kind: RipOut::Triggered, .. })).map(|x| x.0).collect();
    t.eq("r2 triggered", trig.clone(), PROBE_CHAIN_TRIG.to_vec());
    let mut starts: Vec<u64> = trig.clone();
    starts.dedup();
    t.check("holds of 1..5 s", starts.windows(2).all(|w| (SEC..=5 * SEC).contains(&(w[1] - w[0]))));
    // R3's route reached R2 while the hold of its first triggered update ran: it went out when
    // the hold ended, and R1 had it 770 ns later (an update with one route)
    t.eq("after the hold", r1_learned, trig.get(2).map(|t2| t2 + 770));
    let rp = e.rip(r[0]);
    t.eq("next update", rp.and_then(|x| x.next_update()), Some(PROBE_CHAIN_NEXT));
    t.eq("sources", rp.map(|x| x.sources().keys().copied().collect::<Vec<_>>()), Some(alloc::vec![ip("10.0.12.2")]));
    // the RIB forwards with it: a ping from R1's loopback to R3's
    let job = e.ping(r[0], ip("10.3.3.3"), PingOpts { count: 1, src: Some(ip("10.1.1.1")), ..PingOpts::default() }).unwrap_or(0);
    run_to(&mut e, 42 * SEC);
    t.check("ping over two hops", matches!(e.job(job).map(|j| &j.kind), Some(jfn_sim::JobKind::Ping(pg)) if pg.events.iter().any(|ev| matches!(ev, jfn_sim::PingEv::Reply { .. }))));
}

fn split_horizon(t: &mut T) {
    let (p, r, l) = chain();
    let mut e = engine(&p);
    run_to(&mut e, 70 * SEC);
    // R2 tells R1 what it learned from R1 with metric 16 (poisoned reverse), and leaves out the
    // network of that interface
    let to_r1 = rip_frames(&e, l[0], r[1]);
    let last = to_r1.iter().rev().find(|m| m.1.command == rip::CMD_RESPONSE && m.1.entries.len() == 3).map(|m| entries(&m.1));
    t.eq("r2 to r1", last, Some(alloc::vec![(s("10.0.23.0/24"), 1), (s("10.1.1.0/24"), 16), (s("10.3.3.0/24"), 2)]));
    let to_r3 = rip_frames(&e, l[1], r[1]);
    let last = to_r3.iter().rev().find(|m| m.1.command == rip::CMD_RESPONSE && m.1.entries.len() == 3).map(|m| entries(&m.1));
    t.eq("r2 to r3", last, Some(alloc::vec![(s("10.0.12.0/24"), 1), (s("10.1.1.0/24"), 2), (s("10.3.3.0/24"), 16)]));
    t.check("notes count the poisoned", rip_notes(&e, r[1]).iter().any(|x| matches!(&x.1, Note::RipSent { kind: RipOut::Regular, poisoned: 1, iface: 0, .. })));
    // R1 takes nothing from the poison: its own network stays its own
    t.eq("r1 keeps its loopback", best(&e, r[0], "10.1.1.0/24"), alloc::vec![("C", None)]);
}

fn cable_pulled(t: &mut T) {
    let (p, r, l) = chain();
    let mut e = engine(&p);
    run_to(&mut e, 40 * SEC);
    let _ = e.apply(Action::Unplug { link: l[1], down: true });
    run_to(&mut e, 40 * SEC + 10 * MS);
    // R2 loses the path at once and poisons the route in a triggered update to R1
    t.eq("r2 down", when(&e, r[1], |n| matches!(n, Note::RipDown { prefix, why: RipLoss::Iface, .. } if *prefix == pf("10.3.3.0/24"))), Some(40 * SEC));
    let poison = rip_frames(&e, l[0], r[1]).into_iter().find(|m| m.0 >= 40 * SEC).map(|m| (m.0, entries(&m.1)));
    t.eq("triggered poison", poison, Some((40 * SEC, alloc::vec![(s("10.0.23.0/24"), 16), (s("10.3.3.0/24"), 16)])));
    // R1 hears 16 from its next hop: unreachable, held down
    // (an update with two routes: 86 bytes, 880 + 50 ns)
    let down = when(&e, r[0], |n| matches!(n, Note::RipDown { prefix, why: RipLoss::Poisoned, .. } if *prefix == pf("10.3.3.0/24")));
    t.eq("r1 down", down, Some(40 * SEC + 930));
    t.eq("r1 rib empty of it", best(&e, r[0], "10.3.3.0/24"), Vec::new());
    let db = cli(&mut e, r[0], &["show ip rip database"]);
    t.check("database shows the holddown", db.contains("10.3.3.0/24    unreachable (metric 16): held down 180 s more, removed in 120 s\n"));
    // the unreachable route goes out with 16 until the garbage collection removes it
    run_to(&mut e, 159 * SEC);
    let r1_out = rip_frames(&e, l[0], r[0]);
    t.check("still advertised with 16", r1_out.iter().any(|m| m.0 > 41 * SEC && entries(&m.1).contains(&(s("10.3.3.0/24"), 16))));
    run_to(&mut e, 161 * SEC);
    t.eq("r2 flushed", when(&e, r[1], |n| matches!(n, Note::RipFlushed { prefix } if *prefix == pf("10.3.3.0/24"))), Some(160 * SEC));
    t.eq("r1 flushed", when(&e, r[0], |n| matches!(n, Note::RipFlushed { prefix } if *prefix == pf("10.3.3.0/24"))), Some(160 * SEC + 930));
    t.check("gone from the table", e.rip(r[0]).is_some_and(|x| !x.table().contains_key(&pf("10.3.3.0/24"))));
    // plugged in again: R2 greets R3 and the route comes back at once
    let _ = e.apply(Action::Unplug { link: l[1], down: false });
    run_to(&mut e, 167 * SEC);
    t.eq("back", best(&e, r[0], "10.3.3.0/24"), alloc::vec![("R", Some(ip("10.0.12.2")))]);
}

fn silent_neighbour(t: &mut T) {
    // R1 and R2 on a switch: R2's cable to the switch goes, R1's interface stays up
    let mut p = Project::new("rip timeout", 33);
    let r1 = p.add_device("jf-r4", 0, 0).unwrap_or(0);
    let sw = p.add_device("jf-s24p", 200, 0).unwrap_or(0);
    let r2 = p.add_device("jf-r4", 400, 0).unwrap_or(0);
    wire(&mut p, r1, "gi0/0/0", sw, "gi1/0/1");
    let l2 = wire(&mut p, sw, "gi1/0/2", r2, "gi0/0/0");
    set(&mut p, r1, cfg("R1", &[("GigabitEthernet0/0/0", "10.0.12.1/24"), ("Loopback0", "10.1.1.1/24")], RIP2));
    set(&mut p, r2, cfg("R2", &[("GigabitEthernet0/0/0", "10.0.12.2/24"), ("Loopback0", "10.2.2.2/24")], RIP2));
    let mut e = engine(&p);
    run_to(&mut e, 40 * SEC);
    let last = e.rip(r1).and_then(|x| x.sources().get(&ip("10.0.12.2")).copied()).unwrap_or(0);
    t.eq("last update from r2", last, PROBE_TIMEOUT_LAST);
    let _ = e.apply(Action::Unplug { link: l2, down: true });
    run_to(&mut e, last + 179 * SEC);
    t.eq("still there", best(&e, r1, "10.2.2.0/24"), alloc::vec![("R", Some(ip("10.0.12.2")))]);
    run_to(&mut e, last + 181 * SEC);
    t.eq("timed out", when(&e, r1, |n| matches!(n, Note::RipDown { why: RipLoss::Timeout(180), .. })), Some(last + 180 * SEC));
    t.eq("out of the rib", best(&e, r1, "10.2.2.0/24"), Vec::new());
    run_to(&mut e, last + 301 * SEC);
    t.eq("flushed after 300 s", when(&e, r1, |n| matches!(n, Note::RipFlushed { .. })), Some(last + 300 * SEC));
    t.eq("counted", (e.metric(r1, "rip", "timeouts"), e.metric(r1, "rip", "flushed")), (Some(1), Some(1)));
}

fn passive(t: &mut T) {
    // R1 with a PC on Gi0/0/2: first the LAN is passive, then not
    let tail = "router rip\n version 2\n passive-interface GigabitEthernet0/0/2\n network 10.0.0.0\n no auto-summary\n!\n";
    let (mut p, r1, r2, _) = pair(tail, RIP2);
    let pc = p.add_device("pc", 0, 200).unwrap_or(0);
    let lan = wire(&mut p, pc, "eth0", r1, "gi0/0/2");
    let mut c = cfg("R1", &[("GigabitEthernet0/0/0", "10.0.12.1/24"), ("GigabitEthernet0/0/2", "10.0.1.1/24")], tail);
    c = c.replace("hostname R1\n!\n", "hostname R1\n!\ninterface Loopback0\n ip address 10.1.1.1 255.255.255.0\n!\n");
    set(&mut p, r1, c);
    set(&mut p, pc, s("hostname PC1\nip addr add 10.0.1.10/24 dev eth0\nip route add default via 10.0.1.1\n"));
    let mut e = engine(&p);
    run_to(&mut e, 40 * SEC);
    t.eq("no rip on the lan", e.trace.hops().filter(|h| h.link == lan && h.mask & decode::P_RIP != 0).count(), 0);
    t.eq("the lan is advertised", best(&e, r2, "10.0.1.0/24"), alloc::vec![("R", Some(ip("10.0.12.1")))]);
    t.check("passive in the interface list", e.rip(r1).is_some_and(|x| x.ifaces().values().any(|i| i.name == "GigabitEthernet0/0/2" && i.passive)));
    // every interface passive but Gi0/0/0
    let o = cli(&mut e, r1, &["enable", "conf t", "router rip", "passive-interface default", "no passive-interface gi0/0/0", "end"]);
    t.check("accepted", !refused(&o));
    let run = e.running_config(r1);
    t.check("running config", run.contains("router rip\n version 2\n passive-interface default\n no passive-interface GigabitEthernet0/0/0\n network 10.0.0.0\n no auto-summary\n!\n"));
    // the LAN no longer passive: updates reach the PC, which drops them
    let _ = cli(&mut e, r1, &["conf t", "router rip", "no passive-interface default", "end"]);
    run_to(&mut e, 80 * SEC);
    t.check("updates on the lan", e.trace.hops().any(|h| h.link == lan && h.mask & decode::P_RIP != 0));
    t.check("the pc ignores the group", e.trace.events().any(|x| x.dev == pc && x.note == Note::IpGroupIgnored { dst: rip::GROUP }));
}

fn distances(t: &mut T) {
    let (p, r, _) = chain();
    let mut e = engine(&p);
    run_to(&mut e, 10 * SEC);
    // a static route beats RIP (1 against 120); a floating one with 200 does not
    let _ = cli(&mut e, r[0], &["enable", "conf t", "ip route 10.3.3.0 255.255.255.0 10.0.12.2", "end"]);
    t.eq("static wins", best(&e, r[0], "10.3.3.0/24"), alloc::vec![("S", Some(ip("10.0.12.2")))]);
    let shown = cli(&mut e, r[0], &["show ip route"]);
    t.check("show ip route shows the static", shown.contains("S        10.3.3.0/24 [1/0] via 10.0.12.2\n") && !shown.contains("R        10.3.3.0/24"));
    let _ = cli(&mut e, r[0], &["conf t", "no ip route 10.3.3.0 255.255.255.0 10.0.12.2", "ip route 10.3.3.0 255.255.255.0 10.0.12.2 200", "end"]);
    t.eq("rip wins over 200", best(&e, r[0], "10.3.3.0/24"), alloc::vec![("R", Some(ip("10.0.12.2")))]);
    // a distance of 250 makes RIP lose against it
    let _ = cli(&mut e, r[0], &["conf t", "router rip", "distance 250", "end"]);
    t.eq("distance 250", best(&e, r[0], "10.3.3.0/24"), alloc::vec![("S", Some(ip("10.0.12.2")))]);
    t.check("rip route kept with 250", rip_routes(&e, r[0]).iter().any(|x| x.0 == "10.3.3.0/24" && x.3 == 250));
    t.check("running config", e.running_config(r[0]).contains(" network 10.0.0.0\n distance 250\n no auto-summary\n"));
    let _ = cli(&mut e, r[0], &["conf t", "router rip", "no distance", "end"]);
    t.eq("back to 120", best(&e, r[0], "10.3.3.0/24"), alloc::vec![("R", Some(ip("10.0.12.2")))]);
}

fn equal_cost(t: &mut T) {
    // R1 reaches R4's loopback over R2 and over R3 with metric 3
    let mut p = Project::new("rip diamond", 34);
    let r: Vec<u32> = (0..4).map(|i| p.add_device("jf-r4", 200 * i, 0).unwrap_or(0)).collect();
    let l12 = wire(&mut p, r[0], "gi0/0/0", r[1], "gi0/0/0");
    let l13 = wire(&mut p, r[0], "gi0/0/1", r[2], "gi0/0/0");
    wire(&mut p, r[1], "gi0/0/1", r[3], "gi0/0/0");
    wire(&mut p, r[2], "gi0/0/1", r[3], "gi0/0/1");
    set(&mut p, r[0], cfg("R1", &[("GigabitEthernet0/0/0", "10.0.12.1/24"), ("GigabitEthernet0/0/1", "10.0.13.1/24")], RIP2));
    set(&mut p, r[1], cfg("R2", &[("GigabitEthernet0/0/0", "10.0.12.2/24"), ("GigabitEthernet0/0/1", "10.0.24.2/24")], RIP2));
    set(&mut p, r[2], cfg("R3", &[("GigabitEthernet0/0/0", "10.0.13.3/24"), ("GigabitEthernet0/0/1", "10.0.34.3/24")], RIP2));
    set(&mut p, r[3], cfg("R4", &[("GigabitEthernet0/0/0", "10.0.24.4/24"), ("GigabitEthernet0/0/1", "10.0.34.4/24"), ("Loopback0", "10.4.4.4/24")], RIP2));
    let mut e = engine(&p);
    run_to(&mut e, 40 * SEC);
    t.eq("two paths", best(&e, r[0], "10.4.4.0/24"), alloc::vec![("R", Some(ip("10.0.12.2"))), ("R", Some(ip("10.0.13.3")))]);
    let shown = cli(&mut e, r[0], &["enable", "show ip route rip"]);
    t.check("show ip route rip", shown.contains("      10.0.0.0/24 is subnetted, 3 subnets\n") && shown.contains("R        10.4.4.0 [120/3] via 10.0.12.2, ") && shown.contains(" [120/3] via 10.0.13.3, ") && !shown.contains("C        "));
    // one path per destination address: 10.4.4.4 over R2, 10.4.4.5 over R3 (what R1 sends first
    // for them, an ARP request or the echo, goes over that cable)
    t.eq("pick per destination", (jfn_sim::ip::ecmp_pick(ip("10.4.4.4"), 2), jfn_sim::ip::ecmp_pick(ip("10.4.4.5"), 2)), (0, 1));
    let sent_on = |e: &Engine, link: u32, after: u64| e.trace.hops().filter(|h| h.link == link && h.from.0 == r[0] && h.t_tx >= after && h.mask & (decode::P_ICMP | decode::P_ARP) != 0).count();
    let t0 = e.now();
    let _ = e.ping(r[0], ip("10.4.4.4"), PingOpts { count: 1, ..PingOpts::default() });
    run_to(&mut e, t0 + 3 * SEC);
    t.check("10.4.4.4 over r2", sent_on(&e, l12, t0) > 0 && sent_on(&e, l13, t0) == 0);
    let t1 = e.now();
    let _ = e.ping(r[0], ip("10.4.4.5"), PingOpts { count: 1, ..PingOpts::default() });
    run_to(&mut e, t1 + 3 * SEC);
    t.check("10.4.4.5 over r3", sent_on(&e, l13, t1) > 0 && sent_on(&e, l12, t1) == 0);
    // a path lost: the other one stays, without holddown
    let t2 = e.now();
    let _ = e.apply(Action::Unplug { link: l12, down: true });
    run_to(&mut e, t2 + 10 * MS);
    let gone: Vec<Note> = rip_notes(&e, r[0]).into_iter().filter(|x| x.0 == t2 && matches!(x.1, Note::RipPathGone { .. } | Note::RipDown { .. })).map(|x| x.1).collect();
    t.eq(
        "path gone",
        gone,
        alloc::vec![
            Note::RipDown { prefix: pf("10.0.24.0/24"), via: ip("10.0.12.2"), why: RipLoss::Iface },
            Note::RipPathGone { prefix: pf("10.4.4.0/24"), via: ip("10.0.12.2"), why: RipLoss::Iface },
        ],
    );
    t.eq("one path left", best(&e, r[0], "10.4.4.0/24"), alloc::vec![("R", Some(ip("10.0.13.3")))]);
    let _ = e.apply(Action::Unplug { link: l12, down: false });
    run_to(&mut e, t2 + 40 * SEC);
    t.eq("two again", best(&e, r[0], "10.4.4.0/24").len(), 2);
    // maximum-paths 1: the first one only
    let _ = cli(&mut e, r[0], &["conf t", "router rip", "maximum-paths 1", "end"]);
    t.eq("maximum-paths 1", best(&e, r[0], "10.4.4.0/24"), alloc::vec![("R", Some(ip("10.0.12.2")))]);
    t.check("in the running config", e.running_config(r[0]).contains(" maximum-paths 1\n"));
}

fn versions(t: &mut T) {
    // R1 speaks version 1 and has a network of another class; R2 keeps the default (sends 2,
    // takes both)
    let tail1 = "router rip\n version 1\n network 10.0.0.0\n network 172.16.0.0\n!\n";
    let tail2 = "router rip\n network 10.0.0.0\n!\n";
    let (mut p, r1, r2, l) = pair(tail1, tail2);
    set(&mut p, r1, cfg("R1", &[("GigabitEthernet0/0/0", "10.0.12.1/24"), ("Loopback0", "172.16.1.1/24")], tail1));
    let mut e = engine(&p);
    run_to(&mut e, 10 * SEC);
    // R1 broadcasts version 1 and summarises its class B network; R2 infers the classful mask
    let first = rip_frames(&e, l, r1).into_iter().find(|m| m.1.command == rip::CMD_RESPONSE).map(|m| (m.1.version, entries(&m.1)));
    t.eq("v1 summary", first, Some((1, alloc::vec![(s("172.16.0.0/32"), 1)])));
    t.check("v1 to the broadcast", e.trace.hops().any(|h| h.link == l && h.from.0 == r1 && h.mask & decode::P_RIP != 0 && h.bytes[0..6] == [0xFF; 6]));
    t.eq("r2 learned the class b network", best(&e, r2, "172.16.0.0/16"), alloc::vec![("R", Some(ip("10.0.12.1")))]);
    // R2's version 2 multicasts do not reach R1 (it listens to no group), and R2 does not answer
    // R1's version 1 request since it sends only version 2
    t.check("r1 ignores the group", e.trace.events().any(|x| x.dev == r1 && x.note == Note::IpGroupIgnored { dst: rip::GROUP }));
    t.check("r2 does not answer v1", rip_notes(&e, r2).iter().any(|x| matches!(x.1, Note::RipIgnored { why: RipDrop::NoAnswer, .. })));
    t.eq("r1 knows nothing", rip_routes(&e, r1), Vec::new());
    // R2 sends both versions on that interface: R1 takes the version 1 broadcast and infers /24 from
    // its own interface in 10.0.0.0
    let o = cli(&mut e, r2, &["enable", "conf t", "interface gi0/0/0", "ip rip send version 1 2", "end"]);
    t.check("accepted", !refused(&o));
    run_to(&mut e, 20 * SEC);
    t.eq("r1 learned", rip_routes(&e, r1), alloc::vec![(s("10.2.2.0/24"), Some(ip("10.0.12.2")), 2, 120)]);
    t.check("running config", e.running_config(r2).contains("interface GigabitEthernet0/0/0\n ip address 10.0.12.2 255.255.255.0\n ip rip send version 1 2\n!\n"));
    let proto = cli(&mut e, r2, &["show ip protocols"]);
    t.check("versions in show ip protocols", proto.contains("  Versions: send 2, receive 1 and 2\n") && proto.contains("    GigabitEthernet0/0/0       1 2   1 2   no\n"));
    // R1 takes only version 2 on Gi0/0/0 now: it joins the group, and R2's version 1 broadcasts
    // are refused
    let o = cli(&mut e, r1, &["enable", "conf t", "interface gi0/0/0", "ip rip receive version 2", "end"]);
    t.check("receive setting", !refused(&o) && e.running_config(r1).contains("interface GigabitEthernet0/0/0\n ip address 10.0.12.1 255.255.255.0\n ip rip receive version 2\n!\n"));
    run_to(&mut e, 60 * SEC);
    t.check("v1 now ignored", rip_notes(&e, r1).iter().any(|x| x.0 > 20 * SEC && matches!(x.1, Note::RipIgnored { why: RipDrop::Version, .. })));
}

fn summaries(t: &mut T) {
    // R2 has 172.20.5.0/24 behind it: with auto-summary R1 sees 172.20.0.0/16, without it the /24
    let auto = "router rip\n version 2\n network 10.0.0.0\n network 172.20.0.0\n auto-summary\n!\n";
    let (mut p, r1, r2, _) = pair(RIP2, auto);
    set(&mut p, r2, cfg("R2", &[("GigabitEthernet0/0/0", "10.0.12.2/24"), ("Loopback0", "10.2.2.2/24"), ("Loopback1", "172.20.5.1/24")], auto));
    let mut e = engine(&p);
    run_to(&mut e, 10 * SEC);
    t.eq("summarised", rip_routes(&e, r1).iter().map(|x| x.0.clone()).collect::<Vec<_>>(), alloc::vec![s("10.2.2.0/24"), s("172.20.0.0/16")]);
    let db = cli(&mut e, r2, &["enable", "show ip rip database"]);
    t.check("database names the summaries", db.starts_with("10.0.0.0/8    auto-summary\n10.0.12.0/24    directly connected, GigabitEthernet0/0/0\n") && db.contains("172.20.0.0/16    auto-summary\n172.20.5.0/24    directly connected, Loopback1\n"));
    let _ = cli(&mut e, r2, &["conf t", "router rip", "no auto-summary", "end"]);
    run_to(&mut e, 60 * SEC);
    t.check("the /24 now", rip_routes(&e, r1).iter().any(|x| x.0 == "172.20.5.0/24"));
}

fn default_and_redistribution(t: &mut T) {
    let tail2 = "router rip\n version 2\n redistribute connected\n redistribute static metric 5\n network 10.0.0.0\n default-information originate\n no auto-summary\n!\nip route 192.168.50.0 255.255.255.0 Loopback0\n!\n";
    let (mut p, r1, r2, _) = pair(RIP2, tail2);
    set(&mut p, r2, cfg("R2", &[("GigabitEthernet0/0/0", "10.0.12.2/24"), ("Loopback0", "10.2.2.2/24"), ("Loopback1", "192.168.60.1/24")], tail2));
    let mut e = engine(&p);
    t.eq("boot", e.boot_errors(r2), Vec::new());
    run_to(&mut e, 10 * SEC);
    t.eq(
        "r1 learned",
        rip_routes(&e, r1).iter().map(|x| (x.0.clone(), x.2)).collect::<Vec<_>>(),
        alloc::vec![(s("0.0.0.0/0"), 2), (s("10.2.2.0/24"), 2), (s("192.168.50.0/24"), 6), (s("192.168.60.0/24"), 2)],
    );
    let shown = cli(&mut e, r1, &["enable", "show ip route"]);
    t.check("gateway of last resort", shown.contains("Gateway of last resort is 10.0.12.2 to network 0.0.0.0\n") && shown.contains("R*    0.0.0.0/0 [120/2] via 10.0.12.2, "));
    t.check("r2 does not use its own default", best(&e, r2, "0.0.0.0/0").is_empty());
    let run = e.running_config(r2);
    t.check("running config", run.contains("router rip\n version 2\n redistribute connected\n redistribute static metric 5\n network 10.0.0.0\n default-information originate\n no auto-summary\n!\nip route 192.168.50.0 255.255.255.0 Loopback0\n"));
    // the static route goes: it is poisoned and then forgotten
    let _ = cli(&mut e, r2, &["enable", "conf t", "no ip route 192.168.50.0 255.255.255.0", "end"]);
    run_to(&mut e, 11 * SEC);
    t.eq("withdrawn", best(&e, r1, "192.168.50.0/24"), Vec::new());
}

fn l3_switch(t: &mut T) {
    // MLS1 routes over its VLAN 10 interface; RIP needs "ip routing" there
    let mut p = Project::new("rip mls", 35);
    let r1 = p.add_device("jf-r4", 0, 0).unwrap_or(0);
    let mls = p.add_device("jf-s24l3", 200, 0).unwrap_or(0);
    let l = wire(&mut p, r1, "gi0/0/0", mls, "gi1/0/1");
    set(&mut p, r1, cfg("R1", &[("GigabitEthernet0/0/0", "10.0.10.2/24"), ("Loopback0", "10.1.1.1/24")], RIP2));
    let mut e = engine(&p);
    let o = cli(&mut e, mls, &["enable", "conf t", "router rip"]);
    t.check("routing needed", o.ends_with("leaves it.\n% IP routing not enabled\n") && e.cli_prompt(mls) == "MLS1(config)#");
    let o = cli(&mut e, mls, &["ip routing", "vlan 10", "exit", "interface vlan10", "ip address 10.0.10.1 255.255.255.0", "interface gi1/0/1", "switchport access vlan 10", "interface lo0", "ip address 10.9.9.9 255.255.255.0", "router rip", "version 2", "network 10.0.0.0", "end"]);
    t.check("configured", !refused(&o) && o.contains("%LINEPROTO-5-UPDOWN: line protocol on Vlan10 is up\n") && e.cli_prompt(mls) == "MLS1#");
    run_to(&mut e, 10 * SEC);
    t.eq("r1 learned over the svi", best(&e, r1, "10.9.9.0/24"), alloc::vec![("R", Some(ip("10.0.10.1")))]);
    t.eq("mls learned", best(&e, mls, "10.1.1.0/24"), alloc::vec![("R", Some(ip("10.0.10.2")))]);
    // the last port of VLAN 10 goes: the SVI goes down and with it what RIP learned over it
    let _ = e.apply(Action::Unplug { link: l, down: true });
    run_to(&mut e, 11 * SEC);
    t.check("svi down", rip_notes(&e, mls).iter().any(|x| matches!(&x.1, Note::RipDown { prefix, why: RipLoss::Iface, .. } if *prefix == pf("10.1.1.0/24"))));
    // "no ip routing" takes the process with it
    let _ = cli(&mut e, mls, &["conf t", "no ip routing", "end"]);
    t.check("process gone", !e.running_config(mls).contains("router rip") && e.rip(mls).is_some_and(|x| !x.running()));
}

fn command_line(t: &mut T) {
    let (p, r, _) = chain();
    let mut e = engine(&p);
    run_to(&mut e, 40 * SEC);
    let r1 = r[0];
    let _ = cli(&mut e, r1, &["enable", "conf t", "router rip"]);
    t.eq("router mode", e.cli_prompt(r1), s("R1(config-router)#"));
    let h = e.cli_help(r1, "");
    t.check("router help", h.starts_with("Commands for this routing process:\n") && h.contains(" Run RIP on the interfaces of a network\n") && h.contains(" Send no updates on an interface; it still listens\n") && h.contains("  maximum-paths "));
    t.eq("version help", e.cli_help(r1, "version "), s("  <1-2>  1: broadcasts without masks, 2: multicasts with masks\n\n"));
    t.eq("bad network", cli(&mut e, r1, &["network 224.0.0.0"]), s("% 224.0.0.0 is no network RIP can run on\n"));
    t.eq("bad timers", cli(&mut e, r1, &["timers basic 30 180 180 100"]), s("% The flush time must not be shorter than the invalid time\n"));
    let _ = cli(&mut e, r1, &["timers basic 10 60 60 90", "network 10.1.2.3"]);
    t.check("classful network, timers", e.running_config(r1).contains("router rip\n version 2\n timers basic 10 60 60 90\n network 10.0.0.0\n no auto-summary\n!\n"));
    // a global command leaves the router mode
    let _ = cli(&mut e, r1, &["ip route 192.0.2.0 255.255.255.0 10.0.12.2"]);
    t.eq("fallback", e.cli_prompt(r1), s("R1(config)#"));
    let _ = cli(&mut e, r1, &["no ip route 192.0.2.0 255.255.255.0 10.0.12.2", "end"]);
    // the running configuration boots the same router again
    let text = e.running_config(r1);
    let mut p2 = p.clone();
    set(&mut p2, r1, text.clone());
    let e2 = engine(&p2);
    t.eq("boots without errors", e2.boot_errors(r1), Vec::new());
    t.eq("same configuration", e2.running_config(r1), text);
    t.eq("same timers", e2.rip(r1).map(|x| x.cfg.timers.update), Some(10));
    // show ip route with R, its distance, metric, age and interface
    let shown = cli(&mut e, r1, &["show ip route"]);
    t.check("legend", shown.starts_with("Codes: C - connected network, L - address of this device, S - static route,\n       R - learned by RIP, * - default route\n\n"));
    t.check("r line", shown.contains("R        10.3.3.0/24 [120/3] via 10.0.12.2, 00:00:") && shown.contains(", GigabitEthernet0/0/0\n"));
    let proto = cli(&mut e, r1, &["show ip protocols"]);
    t.check("show ip protocols", proto.starts_with("Routing process \"rip\" (RFC 2453)\n  Updates every 10 seconds, the next one in ") && proto.contains("  Routing for networks:\n    10.0.0.0\n") && proto.contains("    10.0.12.2            120      00:00:"));
    // debug ip rip shows the next update with its routes
    let o = cli(&mut e, r1, &["debug ip rip"]);
    t.eq("debug on", o, s("Debug output for RIP: on\n"));
    t.check("show debugging", cli(&mut e, r1, &["show debugging"]).contains("Debug output for RIP: on\n"));
    let _ = e.take_reports();
    let now = e.now();
    run_to(&mut e, now + 40 * SEC);
    let o = out(&mut e, r1);
    t.check("debug lines", o.contains(": RIP: update v2 out of GigabitEthernet0/0/0 to 224.0.0.9, 3 routes\n") && o.contains("\n      10.1.1.0/24 metric 1\n      10.3.3.0/24 metric 16\n") && o.contains(": RIP: update v2 from 10.0.12.2 on GigabitEthernet0/0/0, 3 routes\n"));
    t.check("no time stamp on route lines", o.lines().filter(|l| l.contains(" metric ") && !l.contains("RIP:")).all(|l| l.starts_with("      ")));
    let _ = cli(&mut e, r1, &["undebug all"]);
    // clear ip route *: the learned routes go and come back from the answers to the requests
    let before = rip_routes(&e, r1).len();
    let o = cli(&mut e, r1, &["clear ip route *"]);
    t.check("cleared", o.is_empty() && rip_notes(&e, r1).iter().any(|x| x.0 == e.now() && matches!(x.1, Note::RipRequest { .. })));
    let now = e.now();
    run_to(&mut e, now + 5 * MS);
    t.eq("relearned", rip_routes(&e, r1).len(), before);
    // no router rip: the routes go at once, the interface settings stay
    let _ = cli(&mut e, r1, &["conf t", "interface gi0/0/0", "ip rip receive version 1 2", "exit", "no router rip", "end"]);
    t.eq("withdrawn", rip_routes(&e, r1), Vec::new());
    let run = e.running_config(r1);
    t.check("block gone, interface line kept", !run.contains("router rip") && run.contains(" ip rip receive version 1 2\n"));
    t.check("stopped", e.services(r1).iter().any(|x| x.kind == "rip" && !x.running && !x.enabled));
}

fn power(t: &mut T) {
    let (p, r, _) = chain();
    let mut e = engine(&p);
    run_to(&mut e, 40 * SEC);
    let _ = e.apply(Action::Power { dev: r[1], on: false });
    run_to(&mut e, 41 * SEC);
    t.check("r1 lost r2", rip_notes(&e, r[0]).iter().any(|x| x.0 == 40 * SEC && matches!(x.1, Note::RipDown { why: RipLoss::Iface, .. })));
    t.check("r2 forgot", e.rip(r[1]).is_some_and(|x| x.table().is_empty() && !x.running()) && rip_routes(&e, r[1]).is_empty());
    // back on: R2 starts again; R1 takes the routes back from the router it had them from,
    // holddown or not
    let _ = e.apply(Action::Power { dev: r[1], on: true });
    run_to(&mut e, 47 * SEC);
    t.eq("back within a hold", best(&e, r[0], "10.3.3.0/24"), alloc::vec![("R", Some(ip("10.0.12.2")))]);
    t.check("configuration kept", e.running_config(r[1]).contains("router rip\n") && e.rip(r[1]).is_some_and(|x| x.running()));
}

fn texts(t: &mut T) {
    let rows: Vec<(&str, &str)> = include_str!("locales/engine.tsv").lines().skip(1).filter(|l| !l.is_empty() && !l.starts_with('#')).filter_map(|l| l.split_once('\t')).collect();
    let en = move |k: &'static str| rows.iter().find(|r| r.0 == k).map_or(k, |r| r.1);
    let name = |i: u16| format!("Gi0/0/{}", i);
    let port = |p: u16| format!("p{}", p);
    let cases: Vec<(Note, &str, &str)> = alloc::vec![
        (
            Note::RipSent { iface: 1, ver: 2, to: rip::GROUP, kind: RipOut::Triggered, routes: alloc::vec![(pf("10.0.3.0/24"), 16)], poisoned: 1 },
            "RIP: ausgelöstes Update (Version 2) über Gi0/0/1 an 224.0.0.9, Routen: 1, davon per Split Horizon vergiftet: 1.",
            "RIP: triggered update (version 2) on Gi0/0/1 to 224.0.0.9, routes: 1, poisoned by split horizon: 1.",
        ),
        (
            Note::RipDown { prefix: pf("10.0.3.0/24"), via: ip("10.0.12.2"), why: RipLoss::Timeout(180) },
            "RIP: 10.0.3.0/24 über 10.0.12.2 ist weg (180 s ohne Update): unerreichbar, wird mit Metrik 16 gemeldet.",
            "RIP: 10.0.3.0/24 via 10.0.12.2 is lost (no update for 180 s): unreachable, advertised with metric 16.",
        ),
        (
            Note::RipIgnored { from: ip("10.0.12.2"), iface: 0, why: RipDrop::Neighbour },
            "RIP: Nachricht von 10.0.12.2 über Gi0/0/0 ignoriert: der Absender liegt in keinem Netz des Interfaces.",
            "RIP: message from 10.0.12.2 on Gi0/0/0 ignored: the sender is on none of the interface's subnets.",
        ),
        (
            Note::RipHeld { prefix: pf("10.0.3.0/24"), from: ip("10.0.14.4"), metric: 4 },
            "RIP: 10.0.3.0/24 im Holddown: das Angebot von 10.0.14.4 mit Metrik 4 wird ignoriert.",
            "RIP: 10.0.3.0/24 is held down: the offer of 10.0.14.4 with metric 4 is ignored.",
        ),
        (Note::IpGroupIgnored { dst: rip::GROUP }, "Multicast an 224.0.0.9 verworfen: kein Interface hört auf diese Gruppe.", "Multicast to 224.0.0.9 dropped: no interface listens to this group."),
    ];
    for (n, de, english) in cases {
        let m = n.message(&port, &name);
        t.eq(&format!("german {:?}", n), m.text(), s(de));
        t.eq(&format!("english {:?}", n), m.text_with(&en), s(english));
    }
    t.eq("layers", (Note::RipFlushed { prefix: pf("10.0.0.0/8") }.layer(), Note::IpGroupIgnored { dst: rip::GROUP }.layer()), (7, 3));
    t.eq("mask", Note::RipLearned { prefix: pf("10.0.0.0/8"), via: ip("10.0.0.1"), metric: 2 }.mask() & decode::P_RIP, decode::P_RIP);
}

fn determinism(t: &mut T) {
    let (p, _, l) = chain();
    let digest = |e: &Engine| -> Vec<(u64, u32, Vec<u8>)> { e.trace.hops().filter(|h| h.mask & decode::P_RIP != 0).map(|h| (h.t_tx, h.link, h.bytes.to_vec())).collect() };
    let mut a = engine(&p);
    let mut b = engine(&p);
    run_to(&mut a, 20 * SEC);
    let mut c = a.clone();
    let _ = a.apply(Action::Unplug { link: l[1], down: true });
    let _ = c.apply(Action::Unplug { link: l[1], down: true });
    run_to(&mut b, 20 * SEC);
    let _ = b.apply(Action::Unplug { link: l[1], down: true });
    for e in [&mut a, &mut b, &mut c] {
        run_to(e, 200 * SEC);
    }
    let (da, db, dc) = (digest(&a), digest(&b), digest(&c));
    t.check("two runs alike", da == db && !da.is_empty());
    t.check("a clone goes on alike", da == dc);
    let notes = |e: &Engine| e.trace.events().filter(|x| x.note.is_rip()).map(|x| (x.t, x.dev, x.note.clone())).collect::<Vec<_>>();
    t.check("the same decisions", notes(&a) == notes(&b));
    t.eq("rip frames", da.len(), PROBE_DET_FRAMES);
}

// values that come from the devices' random numbers (seed and device id), measured once
const PROBE_CHAIN_R1: u64 = 4_282_002_260;
const PROBE_CHAIN_REGULAR: [u64; 2] = [0, 26_613_000_000];
const PROBE_CHAIN_TRIG: [u64; 4] = [1490, 1490, 4_282_001_490, 4_282_001_490];
const PROBE_CHAIN_NEXT: u64 = 57_108_000_000;
const PROBE_TIMEOUT_LAST: u64 = 34_640_001_860;
const PROBE_DET_FRAMES: usize = 31;
