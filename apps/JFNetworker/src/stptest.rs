// stptest -- checks of spanning tree in jfn_sim (group stp of --selftest): off by default (a
// project without it behaves as before), BPDUs byte for byte on the wire, root election and
// roles on a triangle and a square for STP and RSTP, the 802.1D timers (30 s to forwarding, the
// wait for Max Age after an indirect failure), RSTP's proposal and agreement and its recovery
// within microseconds, edge ports (portfast, AutoEdge), topology changes (TCN, the TC flag and
// fast aging; RSTP flushing), STP and RSTP bridges together, BPDU guard, costs and priorities,
// a hub (backup port, shared link), the commands and show output, LEDs and the simulation
// mode, the loop problem, determinism, the English of the new notes
//
// The triangle: SW1 Gi1/0/1 - Gi1/0/1 SW2 Gi1/0/2 - Gi1/0/1 SW3 Gi1/0/2 - Gi1/0/2 SW1, a PC on
// Gi1/0/3 of each (PC1..PC3 = 10.0.0.1..3).  Ids: SW1 = 1, SW2 = 2, SW3 = 3, PC1 = 4 ...; the
// bridge MACs 02:4a:00:00:0N:00 make SW2 better than SW3 at equal priority.  Copper at 1 Gbit/s
// with 50 ns delay: a BPDU (60 bytes) takes 722 ns from switch to switch.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use jfn_model::project::{PortRef, Project};
use jfn_sim::{Action, Engine, Led, Ns, PortRole, PortState, Report, MS, SEC};
use jfn_sim::{Note, ProblemKind};
use netpkt::decode;
use netpkt::stp::{Bpdu, BridgeId, Kind, PortId, Role, Times};
use netpkt::Mac;

use crate::selftest::T;

const BUDGET: usize = 5_000_000;
/// one BPDU from switch to switch: (60 + 24) * 8 ns at 1 Gbit/s and 50 ns on the cable
const HOP: Ns = 722;
/// the time a BPDU takes to serialise: one that waits behind another leaves this much later
const SER: Ns = 672;

fn pr(dev: u32, port: &str) -> PortRef {
    PortRef { dev, port: String::from(port) }
}

fn engine(p: &Project) -> Engine {
    Engine::from_project(p).unwrap_or_else(|_| Engine::new(0))
}

fn sw_cfg(name: &str, mode: &str, extra: &str) -> String {
    let m = if mode.is_empty() { String::new() } else { format!("spanning-tree mode {}\n", mode) };
    format!("hostname {}\n!\n{}{}!\nend\n", name, m, extra)
}

/// the triangle with these startup configurations of SW1..SW3 (and PCs with addresses)
fn triangle(cfgs: [String; 3]) -> (Project, [u32; 3], [u32; 3]) {
    let mut p = Project::new("stp", 3);
    let sw: Vec<u32> = (0..3).map(|i| p.add_device("jf-s24p", i * 200, 0).unwrap_or(0)).collect();
    let pcs: Vec<u32> = (0..3).map(|i| p.add_device("pc", i * 200, 200).unwrap_or(0)).collect();
    let _ = p.connect(&pr(sw[0], "gi1/0/1"), &pr(sw[1], "gi1/0/1"), None);
    let _ = p.connect(&pr(sw[1], "gi1/0/2"), &pr(sw[2], "gi1/0/1"), None);
    let _ = p.connect(&pr(sw[2], "gi1/0/2"), &pr(sw[0], "gi1/0/2"), None);
    for i in 0..3 {
        let _ = p.connect(&pr(pcs[i], "eth0"), &pr(sw[i], "gi1/0/3"), None);
        if let Some(d) = p.device_mut(sw[i]) {
            d.startup_config = cfgs[i].clone();
        }
        if let Some(d) = p.device_mut(pcs[i]) {
            d.startup_config = format!("hostname PC{}\nip addr add 10.0.0.{}/24 dev eth0\n", i + 1, i + 1);
        }
    }
    (p, [sw[0], sw[1], sw[2]], [pcs[0], pcs[1], pcs[2]])
}

/// the triangle with one protocol; SW1 has priority 4096; `edge`: portfast on the PC ports
fn tri(mode: &str, edge: bool) -> (Project, [u32; 3], [u32; 3]) {
    let e = if edge { "interface GigabitEthernet1/0/3\n spanning-tree portfast\n!\n" } else { "" };
    triangle([sw_cfg("SW1", mode, &format!("spanning-tree priority 4096\n!\n{}", e)), sw_cfg("SW2", mode, e), sw_cfg("SW3", mode, e)])
}

/// the decisions of a device with their times
fn notes(e: &Engine, dev: u32) -> Vec<(Ns, Note)> {
    e.trace.events().filter(|x| x.dev == dev).map(|x| (x.t, x.note.clone())).collect()
}

/// when a port of a device changed to a state (all times, in order)
fn state_times(e: &Engine, dev: u32, port: u16, to: PortState) -> Vec<Ns> {
    notes(e, dev).into_iter().filter(|(_, n)| matches!(n, Note::StpState { port: p, to: s, .. } if *p == port && *s == to)).map(|(t, _)| t).collect()
}

/// (role, state) of a port of a device now
fn port_of(e: &Engine, dev: u32, port: u16) -> Option<(PortRole, PortState)> {
    e.stp_view(dev).and_then(|v| v.port(port).map(|p| (p.role, p.state)))
}

/// the BPDUs on the wire so far
fn bpdu_hops(e: &Engine) -> usize {
    e.trace.hops().filter(|h| h.mask & decode::P_STP != 0).count()
}

fn unplug(e: &mut Engine, p: &Project, dev: u32, port: &str) {
    if let Some(l) = p.link_at(dev, port) {
        let _ = e.apply(Action::Unplug { link: l.id, down: true });
    }
}

/// what a device printed since the last call
fn out(e: &mut Engine, dev: u32) -> String {
    let mut s = String::new();
    for r in e.take_reports() {
        if let Report::CliOut { dev: d, bytes, .. } = r
            && d == dev
        {
            s.push_str(&String::from_utf8_lossy(&bytes));
        }
    }
    s
}

/// a line typed at a device, then 10 ms: what it printed
fn cli(e: &mut Engine, dev: u32, line: &str) -> String {
    let _ = e.take_reports();
    let _ = e.cli_line(dev, line);
    let t = e.now() + 10 * MS;
    e.run_until(t, BUDGET);
    out(e, dev)
}

fn lines(e: &mut Engine, dev: u32, ls: &[&str]) -> String {
    let mut o = String::new();
    for l in ls {
        o = cli(e, dev, l);
    }
    o
}

/// a ping from a PC: the summary line of iputils
fn ping(e: &mut Engine, pc: u32, ip: &str, wait: Ns) -> String {
    let _ = e.take_reports();
    let _ = e.cli_line(pc, &format!("ping -c 1 -W 1 {}", ip));
    let t = e.now() + wait;
    e.run_until(t, BUDGET);
    out(e, pc).lines().find(|l| l.contains("packets transmitted")).map(String::from).unwrap_or_default()
}

const OK1: &str = "1 packets transmitted, 1 received, 0% packet loss, time 0ms";

pub fn run(t: &mut T) {
    off_by_default(t);
    on_the_wire(t);
    election(t);
    square(t);
    stp_timers(t);
    rstp_fast(t);
    edge_ports(t);
    topology_change(t);
    mixed(t);
    bpdu_guard(t);
    costs_and_priorities(t);
    hub(t);
    commands(t);
    leds_and_sim_mode(t);
    problems(t);
    determinism(t);
    english(t);
}

// without "spanning-tree mode" a bridge is what it was: no BPDUs, BPDUs stay in it, the loop is
// a problem with the old text, the configuration has no line of it
fn off_by_default(t: &mut T) {
    let (p, sw, pcs) = triangle([sw_cfg("SW1", "", ""), sw_cfg("SW2", "", ""), sw_cfg("SW3", "", "")]);
    let mut e = engine(&p);
    e.run_until(60 * SEC, BUDGET);
    t.check("no tree", sw.iter().all(|&s| e.stp_view(s).is_none()));
    t.eq("no bpdus", bpdu_hops(&e), 0);
    t.check("leds up", e.view().links.iter().all(|l| l.led == [Led::Up, Led::Up]));
    let b = Bpdu::config(0, BridgeId::new(32768, Mac([2, 0, 0, 0, 0, 9])), 0, BridgeId::new(32768, Mac([2, 0, 0, 0, 0, 9])), PortId::new(128, 1), Times { msg_age: 0, max_age: 20, hello: 2, fwd_delay: 15 });
    let _ = e.apply(Action::Inject { dev: pcs[0], port: String::from("eth0"), frame: b.frame(Mac([2, 0, 0, 0, 0, 9])) });
    e.run_until(61 * SEC, BUDGET);
    t.check("bpdu consumed", notes(&e, sw[0]).iter().any(|(_, n)| *n == Note::Consumed { port: 2 }));
    t.eq("no bpdu went on", bpdu_hops(&e), 1);
    let loops: Vec<String> = e.problems().into_iter().filter(|p| p.kind == ProblemKind::Loop).map(|p| p.text).collect();
    t.eq("loop as before", loops, alloc::vec![String::from("Schleife ohne Spanning Tree über SW1, SW2, SW3 (VLAN 1): Broadcasts kreisen endlos.")]);
    t.check("no lines", !e.running_config(sw[0]).contains("spanning-tree"));
    t.eq("show without", lines(&mut e, sw[0], &["enable", "show spanning-tree"]), String::from("Spanning tree is off on this switch; \"spanning-tree mode rstp\" (or stp) turns it on.\n"));
    // settings without a mode stay in the configuration and wait for it
    let o = lines(&mut e, sw[0], &["configure terminal", "interface gi1/0/3", "spanning-tree portfast", "end"]);
    t.check("portfast note", o.is_empty() || !o.contains('^'));
    t.check("portfast kept", e.running_config(sw[0]).contains("interface GigabitEthernet1/0/3\n spanning-tree portfast\n!"));
    t.check("still off", e.stp_view(sw[0]).is_none());
}

// the first BPDUs of both protocols, byte for byte, and how the dissector reads them
fn on_the_wire(t: &mut T) {
    let (p, sw, _) = tri("rstp", false);
    let e = engine(&p);
    let first = e.trace.hops().find(|h| h.from == (sw[0], 0));
    let f: Vec<u8> = first.map(|h| h.bytes.to_vec()).unwrap_or_default();
    let mut want: Vec<u8> = alloc::vec![0x01, 0x80, 0xC2, 0, 0, 0, 0x02, 0x4A, 0, 0, 0x01, 0x01, 0x00, 0x27, 0x42, 0x42, 0x03];
    // RST BPDU: designated port, proposal; root and bridge 4096/02:4a:00:00:01:00, port 128.1
    want.extend_from_slice(&[0, 0, 2, 2, 0x0E, 0x10, 0, 0x02, 0x4A, 0, 0, 0x01, 0, 0, 0, 0, 0, 0x10, 0, 0x02, 0x4A, 0, 0, 0x01, 0, 0x80, 0x01, 0, 0, 0x14, 0, 0x02, 0, 0x0F, 0, 0]);
    want.resize(60, 0);
    t.eq("rst bpdu bytes", f.clone(), want);
    t.eq("rst summary", decode::info(&f).1, String::from("RST. Root = 4096/0/02:4a:00:00:01:00  Cost = 0  Port = 0x8001"));
    t.check("stp bit", first.is_some_and(|h| h.mask & decode::P_STP != 0 && h.t_tx == 0));
    let (p2, sw2, _) = tri("stp", false);
    let e2 = engine(&p2);
    let c = e2.trace.hops().find(|h| h.from == (sw2[1], 0)).map(|h| h.bytes.to_vec()).unwrap_or_default();
    t.eq("config bpdu", netpkt::stp::of_frame(&c).map(|b| (b.kind, b.flags, b.root, b.cost, b.port, b.msg_age)), Some((Kind::Config, 0, BridgeId::new(32768, Mac([2, 0x4A, 0, 0, 2, 0])), 0, PortId::new(128, 1), 0)));
    t.eq("config length field", (c.len(), c[12], c[13]), (60, 0x00, 0x26));
}

// root election and roles on the triangle, with both protocols
fn election(t: &mut T) {
    for mode in ["stp", "rstp"] {
        let (p, sw, _) = tri(mode, true);
        let mut e = engine(&p);
        e.run_until(40 * SEC, BUDGET);
        let v1 = e.stp_view(sw[0]);
        t.check(&format!("{}: sw1 is the root", mode), v1.as_ref().is_some_and(|v| v.is_root() && v.root_cost == 0 && v.ports.iter().all(|x| x.role == PortRole::Designated)));
        let v3 = e.stp_view(sw[2]);
        t.eq(&format!("{}: sw3 root and cost", mode), v3.as_ref().map(|v| (v.root, v.root_port, v.root_cost)), Some((BridgeId::new(4096, Mac([2, 0x4A, 0, 0, 1, 0])), Some(1), 20_000)));
        let blocked = if mode == "stp" { PortState::Blocking } else { PortState::Discarding };
        t.eq(&format!("{}: sw2 ports", mode), (port_of(&e, sw[1], 0), port_of(&e, sw[1], 1)), (Some((PortRole::Root, PortState::Forwarding)), Some((PortRole::Designated, PortState::Forwarding))));
        t.eq(&format!("{}: sw3 ports", mode), (port_of(&e, sw[2], 0), port_of(&e, sw[2], 1)), (Some((PortRole::Alternate, blocked)), Some((PortRole::Root, PortState::Forwarding))));
        // exactly one port of the circle does not forward
        let off: usize = sw.iter().map(|&s| e.stp_view(s).map_or(0, |v| v.ports.iter().filter(|x| !x.state.forwards()).count())).sum();
        t.eq(&format!("{}: one blocked port", mode), off, 1);
        t.eq(&format!("{}: costs by speed", mode), e.stp_view(sw[1]).map(|v| v.ports.iter().map(|x| x.cost).collect::<Vec<_>>()), Some(alloc::vec![20_000, 20_000, 20_000]));
    }
}

// four switches in a square with equal priorities: the lowest MAC is the root, the opposite
// switch has two equal paths and takes the one over the lower bridge ID
fn square(t: &mut T) {
    for mode in ["stp", "rstp"] {
        let mut p = Project::new("square", 5);
        let sw: Vec<u32> = (0..4).map(|i| p.add_device("jf-s24p", i * 200, 0).unwrap_or(0)).collect();
        for i in 0..4 {
            let _ = p.connect(&pr(sw[i], "gi1/0/1"), &pr(sw[(i + 1) % 4], "gi1/0/2"), None);
            if let Some(d) = p.device_mut(sw[i]) {
                d.startup_config = sw_cfg(&format!("SW{}", i + 1), mode, "");
            }
        }
        let mut e = engine(&p);
        e.run_until(40 * SEC, BUDGET);
        // SW1 Gi1/0/1 - Gi1/0/2 SW2 Gi1/0/1 - Gi1/0/2 SW3 Gi1/0/1 - Gi1/0/2 SW4 Gi1/0/1 - Gi1/0/2 SW1
        t.check(&format!("{}: lowest mac is the root", mode), e.stp_view(sw[0]).is_some_and(|v| v.is_root()));
        t.eq(&format!("{}: sw2 root port", mode), e.stp_view(sw[1]).map(|v| (v.root_port, v.root_cost)), Some((Some(1), 20_000)));
        t.eq(&format!("{}: sw4 root port", mode), e.stp_view(sw[3]).map(|v| (v.root_port, v.root_cost)), Some((Some(0), 20_000)));
        t.eq(&format!("{}: sw3 two paths", mode), e.stp_view(sw[2]).map(|v| (v.root_port, v.root_cost)), Some((Some(1), 40_000)));
        t.eq(&format!("{}: sw3 to sw4 blocked", mode), port_of(&e, sw[2], 0).map(|x| x.0), Some(PortRole::Alternate));
        let off: usize = sw.iter().map(|&s| e.stp_view(s).map_or(0, |v| v.ports.iter().filter(|x| !x.state.forwards()).count())).sum();
        t.eq(&format!("{}: one blocked port", mode), off, 1);
    }
}

// 802.1D: Listening, Learning after 15 s, Forwarding after 30 s; a direct failure takes 30 s,
// an indirect one first waits for the stored information to age out
fn stp_timers(t: &mut T) {
    let (p, sw, pcs) = tri("stp", false);
    let mut e = engine(&p);
    e.run_until(40 * SEC, BUDGET);
    t.eq("listening at boot", state_times(&e, sw[1], 0, PortState::Listening), alloc::vec![0]);
    t.eq("learning after 15 s", state_times(&e, sw[1], 0, PortState::Learning), alloc::vec![15 * SEC]);
    t.eq("forwarding after 30 s", state_times(&e, sw[1], 0, PortState::Forwarding), alloc::vec![30 * SEC]);
    t.eq("a host port too", state_times(&e, sw[0], 2, PortState::Forwarding), alloc::vec![30 * SEC]);
    // SW2's BPDU relayed after its hold time makes SW3's port to it an alternate one at 1 s
    t.eq("blocked after the hold time", state_times(&e, sw[2], 0, PortState::Blocking), alloc::vec![SEC + HOP]);
    let mut e0 = engine(&p);
    e0.run_until(29 * SEC, BUDGET);
    t.check("no way before 30 s", ping(&mut e0, pcs[1], "10.0.0.3", 2 * SEC).contains("100% packet loss"));
    t.eq("a way at 30 s", ping(&mut e, pcs[1], "10.0.0.3", SEC), String::from(OK1));
    // indirect failure: SW1 - SW2 pulled at 40 s; SW2 claims the root at once, SW3 ignores its
    // inferior BPDUs until the information from SW2 ages out: the last one left SW2 at 38 s
    // (relayed from the root's BPDU of 38 s) with message age 1, Max Age 20: 38 + 19 = 57 s
    let mut e2 = engine(&p);
    e2.run_until(40 * SEC, BUDGET);
    unplug(&mut e2, &p, sw[0], "gi1/0/1");
    e2.run_until(100 * SEC, BUDGET);
    let root_now = notes(&e2, sw[1]).into_iter().find(|(t0, n)| *t0 >= 40 * SEC && matches!(n, Note::StpRoot { port: None, .. })).map(|x| x.0);
    t.eq("sw2 thinks it is the root", root_now, Some(40 * SEC));
    let aged = notes(&e2, sw[2]).into_iter().find(|(_, n)| *n == Note::StpAged { port: 0 }).map(|x| x.0);
    t.eq("aged out after max age", aged, Some(57 * SEC));
    t.eq("listening then", state_times(&e2, sw[2], 0, PortState::Listening).last().copied(), Some(57 * SEC));
    let fwd = state_times(&e2, sw[2], 0, PortState::Forwarding).last().copied();
    t.eq("forwarding 47 s after the failure", fwd, Some(87 * SEC));
    t.check("at most max age + 2 x forward delay", fwd.is_some_and(|f| f - 40 * SEC <= 50 * SEC));
    t.eq("sw2 root port over sw3", e2.stp_view(sw[1]).map(|v| (v.root_port, v.root_cost)), Some((Some(1), 40_000)));
    t.eq("pc2 reaches pc1 again", ping(&mut e2, pcs[1], "10.0.0.1", 3 * SEC), String::from(OK1));
    // direct failure: SW1 - SW3 pulled at 40 s; SW3's alternate port becomes its root port and
    // goes through Listening and Learning
    let mut e3 = engine(&p);
    e3.run_until(40 * SEC, BUDGET);
    unplug(&mut e3, &p, sw[0], "gi1/0/2");
    e3.run_until(75 * SEC, BUDGET);
    t.eq("direct: listening at once", state_times(&e3, sw[2], 0, PortState::Listening).last().copied(), Some(40 * SEC));
    t.eq("direct: forwarding 30 s later", state_times(&e3, sw[2], 0, PortState::Forwarding).last().copied(), Some(70 * SEC));
    t.eq("direct: new root port", e3.stp_view(sw[2]).map(|v| v.root_port), Some(Some(0)));
}

// RSTP: proposal and agreement at boot and after a failure, all within microseconds
fn rstp_fast(t: &mut T) {
    let (p, sw, pcs) = tri("rstp", true);
    let mut e = engine(&p);
    e.run_until(SEC, BUDGET);
    // SW1 proposes on Gi1/0/1; SW2's root port forwards with the first BPDU and agrees
    t.eq("root port forwards at once", state_times(&e, sw[1], 0, PortState::Forwarding), alloc::vec![HOP]);
    let agree = notes(&e, sw[1]).into_iter().find(|(_, n)| matches!(n, Note::StpTx { port: 0, bpdu } if bpdu.agreement() && bpdu.role() == Role::Root)).map(|x| x.0);
    t.eq("sw2 agrees", agree, Some(HOP));
    let proposal = notes(&e, sw[0]).into_iter().find(|(_, n)| matches!(n, Note::StpTx { port: 0, bpdu } if bpdu.proposal())).map(|x| x.0);
    t.eq("sw1 proposed at boot", proposal, Some(0));
    t.eq("sw1 forwards on the agreement", state_times(&e, sw[0], 0, PortState::Forwarding), alloc::vec![2 * HOP]);
    t.eq("sw3 alternate", port_of(&e, sw[2], 0), Some((PortRole::Alternate, PortState::Discarding)));
    t.eq("a way at 1 s", ping(&mut e, pcs[1], "10.0.0.3", SEC), String::from(OK1));
    // indirect failure at 40 s: SW2 claims the root (its BPDU waits behind the periodic one it
    // sent at 40 s), SW3's port takes the news as designated port and proposes, SW2 agrees:
    // forwarding two hops later
    let mut e2 = engine(&p);
    e2.run_until(40 * SEC, BUDGET);
    unplug(&mut e2, &p, sw[0], "gi1/0/1");
    e2.run_until(41 * SEC, BUDGET);
    t.eq("sw3 port designated", notes(&e2, sw[2]).into_iter().filter(|(_, n)| *n == Note::StpRole { port: 0, from: PortRole::Alternate, to: PortRole::Designated }).map(|x| x.0).collect::<Vec<_>>(), alloc::vec![40 * SEC + SER + HOP]);
    t.eq("forwarding within microseconds", state_times(&e2, sw[2], 0, PortState::Forwarding).last().copied(), Some(40 * SEC + SER + 3 * HOP));
    t.eq("sw2 over sw3", e2.stp_view(sw[1]).map(|v| (v.root_port, v.root_cost)), Some((Some(1), 40_000)));
    t.eq("pc2 reaches pc1 at once", ping(&mut e2, pcs[1], "10.0.0.1", 2 * SEC), String::from(OK1));
    // direct failure: SW3's alternate port becomes the root port and forwards in that instant
    let mut e3 = engine(&p);
    e3.run_until(40 * SEC, BUDGET);
    unplug(&mut e3, &p, sw[0], "gi1/0/2");
    e3.run_until(41 * SEC, BUDGET);
    t.eq("direct: forwarding at once", state_times(&e3, sw[2], 0, PortState::Forwarding).last().copied(), Some(40 * SEC));
    t.eq("direct: root port", e3.stp_view(sw[2]).map(|v| v.root_port), Some(Some(0)));
}

// portfast ports forward at once and make no topology change; RSTP finds edge ports by
// itself after 3 s (AutoEdge); a BPDU ends an edge port
fn edge_ports(t: &mut T) {
    for mode in ["stp", "rstp"] {
        let (p, sw, _) = tri(mode, true);
        let mut e = engine(&p);
        e.run_until(10 * SEC, BUDGET);
        t.eq(&format!("{}: portfast forwards at boot", mode), state_times(&e, sw[0], 2, PortState::Forwarding), alloc::vec![0]);
        t.check(&format!("{}: edge in show", mode), e.stp_view(sw[0]).is_some_and(|v| v.port(2).is_some_and(|x| x.edge)));
        t.check(&format!("{}: no topology change from it", mode), !notes(&e, sw[0]).iter().any(|(_, n)| matches!(n, Note::StpTopology { port: Some(2), .. })));
    }
    let (p, sw, pcs) = tri("rstp", false);
    let mut e = engine(&p);
    e.run_until(10 * SEC, BUDGET);
    t.eq("rstp: auto edge after 3 s", state_times(&e, sw[0], 2, PortState::Forwarding), alloc::vec![3 * SEC]);
    t.check("rstp: auto edge note", notes(&e, sw[0]).iter().any(|(t0, n)| *t0 == 3 * SEC && *n == Note::StpEdge { port: 2, edge: true }));
    // a BPDU on an edge port: it is a normal port again
    let me = Mac([2, 0x4A, 0, 0, pcs[0] as u8, 1]);
    let b = Bpdu::rst(0, BridgeId::new(61440, me), 0, BridgeId::new(61440, me), PortId::new(128, 1), Times { msg_age: 0, max_age: 20, hello: 2, fwd_delay: 15 }).with_role(Role::Designated);
    let _ = e.apply(Action::Inject { dev: pcs[0], port: String::from("eth0"), frame: b.frame(me) });
    e.run_until(11 * SEC, BUDGET);
    t.check("bpdu ends the edge", notes(&e, sw[0]).iter().any(|(_, n)| *n == Note::StpEdge { port: 2, edge: false }));
    t.check("still designated", port_of(&e, sw[0], 2).is_some_and(|x| x.0 == PortRole::Designated));
}

// 802.1D: TCN up to the root, its acknowledgment, the TC flag for 35 s and fast aging;
// RSTP: TC flags and MAC entries flushed
fn topology_change(t: &mut T) {
    let (p, sw, pcs) = tri("stp", true);
    let mut e = engine(&p);
    e.run_until(32 * SEC, BUDGET);
    let n2 = notes(&e, sw[1]);
    t.check("tc detected at 30 s", n2.iter().any(|(t0, n)| *t0 == 30 * SEC && *n == Note::StpTopology { port: Some(0), what: jfn_sim::stp::TcWhat::Detected }));
    t.check("tcn to the root", n2.iter().any(|(t0, n)| *t0 == 30 * SEC && matches!(n, Note::StpTx { port: 0, bpdu } if bpdu.kind == Kind::Tcn)));
    t.check("root got it", notes(&e, sw[0]).iter().any(|(_, n)| *n == Note::StpTopology { port: Some(0), what: jfn_sim::stp::TcWhat::TcnReceived }));
    t.check("root sets the flag", notes(&e, sw[0]).iter().any(|(_, n)| *n == Note::StpTopology { port: None, what: jfn_sim::stp::TcWhat::RootFlag(true) }));
    // the root's acknowledgment waits for its hold time (it sent at 30 s)
    t.check("acknowledged at 31 s", n2.iter().any(|(t0, n)| *t0 == 31 * SEC + HOP && *n == Note::StpTopology { port: Some(0), what: jfn_sim::stp::TcWhat::Acked }));
    t.eq("fast aging", e.bridge(sw[1]).map(|b| b.age_limit()), Some(15 * SEC));
    t.eq("ping learns", ping(&mut e, pcs[0], "10.0.0.2", SEC), String::from(OK1));
    t.check("learned", e.bridge(sw[1]).is_some_and(|b| b.entries(e.now()).len() == 2));
    e.run_until(50 * SEC, BUDGET);
    t.check("aged after 15 s", e.bridge(sw[1]).is_some_and(|b| b.entries(e.now()).is_empty()));
    e.run_until(70 * SEC, BUDGET);
    // the root's TC timer: Max Age + Forward Delay from 30 s; the others with its next BPDU
    t.check("flag ends at 65 s", notes(&e, sw[0]).iter().any(|(t0, n)| *t0 == 65 * SEC && *n == Note::StpTopology { port: None, what: jfn_sim::stp::TcWhat::RootFlag(false) }));
    t.check("normal aging at 66 s", notes(&e, sw[1]).iter().any(|(t0, n)| *t0 == 66 * SEC + HOP && *n == Note::StpAging { secs: 300, fast: false }));
    t.check("aged entries stay out", e.bridge(sw[1]).is_some_and(|b| b.age_limit() == 300 * SEC && b.entries(e.now()).is_empty()));
    // RSTP: the TC after the failure flushes what was learned
    let (p2, sw2, pcs2) = tri("rstp", true);
    let mut e2 = engine(&p2);
    e2.run_until(10 * SEC, BUDGET);
    t.eq("rstp ping", ping(&mut e2, pcs2[2], "10.0.0.2", SEC), String::from(OK1));
    let before = e2.bridge(sw2[2]).map(|b| b.entries(e2.now()).len());
    unplug(&mut e2, &p2, sw2[0], "gi1/0/1");
    e2.run_until(12 * SEC, BUDGET);
    let flushed: u32 = notes(&e2, sw2[2]).iter().filter_map(|(_, n)| match n {
        Note::StpFlush { entries, .. } => Some(*entries),
        _ => None,
    }).sum();
    t.eq("sw3 knew two", before, Some(2));
    t.check("sw3 flushed", flushed >= 1);
    t.check("tc flag sent", notes(&e2, sw2[2]).iter().any(|(_, n)| matches!(n, Note::StpTx { bpdu, .. } if bpdu.tc())));
    t.eq("still reachable", ping(&mut e2, pcs2[2], "10.0.0.2", SEC), String::from(OK1));
}

// SW2 speaks 802.1D, SW1 and SW3 RSTP: the RSTP ports towards SW2 fall back to Configuration
// BPDUs, SW2 drops the RST BPDUs, the tree is the same
fn mixed(t: &mut T) {
    let (p, sw, pcs) = triangle([sw_cfg("SW1", "rstp", "spanning-tree priority 4096\n"), sw_cfg("SW2", "stp", ""), sw_cfg("SW3", "rstp", "")]);
    let mut e = engine(&p);
    e.run_until(40 * SEC, BUDGET);
    t.check("sw2 drops rst bpdus", notes(&e, sw[1]).iter().any(|(_, n)| matches!(n, Note::StpDropped { why: jfn_sim::stp::BpduDrop::Type, .. })));
    let peer = |e: &Engine, d: u32, port: u16| e.stp_view(d).and_then(|v| v.port(port).map(|x| x.stp_peer));
    t.eq("sw1 towards sw2 speaks stp", (peer(&e, sw[0], 0), peer(&e, sw[0], 1)), (Some(true), Some(false)));
    t.eq("sw3 towards sw2 speaks stp", (peer(&e, sw[2], 0), peer(&e, sw[2], 1)), (Some(true), Some(false)));
    t.check("migration note", notes(&e, sw[0]).iter().any(|(_, n)| *n == Note::StpMigrate { port: 0, rstp: false }));
    t.eq("sw2 root port", e.stp_view(sw[1]).map(|v| (v.root, v.root_port)), Some((BridgeId::new(4096, Mac([2, 0x4A, 0, 0, 1, 0])), Some(0))));
    t.eq("sw3 blocked towards sw2", port_of(&e, sw[2], 0), Some((PortRole::Alternate, PortState::Discarding)));
    // SW1's designated port towards SW2 waits fdWhile (Max Age), then Forward Delay to learn
    t.eq("sw1 learns at 20 s", state_times(&e, sw[0], 0, PortState::Learning), alloc::vec![20 * SEC]);
    t.eq("sw1 forwards at 35 s", state_times(&e, sw[0], 0, PortState::Forwarding), alloc::vec![35 * SEC]);
    t.eq("sw2 forwards at 30 s", state_times(&e, sw[1], 0, PortState::Forwarding), alloc::vec![30 * SEC]);
    t.eq("pc2 reaches pc1", ping(&mut e, pcs[1], "10.0.0.1", SEC), String::from(OK1));
    // clear spanning-tree detected-protocols: RSTP is tried again; SW2's root port sends no
    // Configuration BPDU and drops the RST BPDUs, so only when its information ages out (Max
    // Age) and it speaks as a designated port does SW1 fall back again
    let o = lines(&mut e, sw[0], &["enable", "clear spanning-tree detected-protocols"]);
    t.eq("clear quietly", o, String::new());
    t.check("rstp again", notes(&e, sw[0]).iter().any(|(_, n)| *n == Note::StpMigrate { port: 0, rstp: true }));
    e.run_until(e.now() + 10 * SEC, BUDGET);
    t.eq("rstp for a while", peer(&e, sw[0], 0), Some(false));
    e.run_until(e.now() + 20 * SEC, BUDGET);
    t.eq("and back to stp", peer(&e, sw[0], 0), Some(true));
    t.eq("still reachable", ping(&mut e, pcs[1], "10.0.0.1", SEC), String::from(OK1));
}

// BPDU guard: a BPDU shuts the port off until shutdown and no shutdown; it works without
// spanning tree too
fn bpdu_guard(t: &mut T) {
    let (p, sw, pcs) = tri("rstp", true);
    let mut e = engine(&p);
    e.run_until(5 * SEC, BUDGET);
    lines(&mut e, sw[0], &["enable", "configure terminal", "interface gi1/0/3", "spanning-tree bpduguard enable", "end"]);
    t.check("guard in the config", e.running_config(sw[0]).contains(" spanning-tree portfast\n spanning-tree bpduguard enable\n!"));
    let me = Mac([2, 0x4A, 0, 0, pcs[0] as u8, 1]);
    let b = Bpdu::config(0, BridgeId::new(32768, me), 0, BridgeId::new(32768, me), PortId::new(128, 1), Times { msg_age: 0, max_age: 20, hello: 2, fwd_delay: 15 });
    let _ = e.take_reports();
    let _ = e.apply(Action::Inject { dev: pcs[0], port: String::from("eth0"), frame: b.frame(me) });
    e.run_until(6 * SEC, BUDGET);
    let o = out(&mut e, sw[0]);
    t.check("console says so", o.contains("%SPANTREE-2-BLOCK_BPDUGUARD: a BPDU arrived on GigabitEthernet1/0/3, where BPDU guard is on: the port is shut off") && o.contains("%PM-4-ERR_DISABLE: GigabitEthernet1/0/3 is err-disabled"));
    t.check("link down too", o.contains("%LINK-3-UPDOWN: GigabitEthernet1/0/3 is down"));
    t.check("err-disabled", e.node(sw[0]).is_some_and(|n| n.ports[2].err_disabled));
    t.check("note", notes(&e, sw[0]).iter().any(|(_, n)| *n == Note::StpErrDisable { port: 2 }));
    t.eq("led", e.port_led(sw[0], 2), Led::Down);
    t.check("show interfaces", cli(&mut e, sw[0], "show interfaces gi1/0/3").starts_with("GigabitEthernet1/0/3 is down, line protocol is down (err-disabled)\n"));
    lines(&mut e, sw[0], &["configure terminal", "interface gi1/0/3", "no shutdown", "end"]);
    t.check("no shutdown alone keeps it", e.node(sw[0]).is_some_and(|n| n.ports[2].err_disabled) && e.port_led(sw[0], 2) == Led::Down);
    lines(&mut e, sw[0], &["configure terminal", "interface gi1/0/3", "shutdown", "no shutdown", "end"]);
    e.run_until(e.now() + SEC, BUDGET);
    t.check("shutdown, no shutdown", e.node(sw[0]).is_some_and(|n| !n.ports[2].err_disabled) && e.port_led(sw[0], 2) == Led::Up);
    t.eq("pc1 back", ping(&mut e, pcs[0], "10.0.0.2", SEC), String::from(OK1));
    // without spanning tree
    let (p2, sw2, pcs2) = triangle([sw_cfg("SW1", "", "interface GigabitEthernet1/0/3\n spanning-tree bpduguard enable\n!\n"), sw_cfg("SW2", "", ""), sw_cfg("SW3", "", "")]);
    let mut e2 = engine(&p2);
    let _ = e2.apply(Action::Inject { dev: pcs2[0], port: String::from("eth0"), frame: b.frame(me) });
    e2.run_until(SEC, BUDGET);
    t.check("guard without spanning tree", e2.node(sw2[0]).is_some_and(|n| n.ports[2].err_disabled));
}

// costs, priorities and the speed of links decide the tree
fn costs_and_priorities(t: &mut T) {
    for mode in ["stp", "rstp"] {
        let (p, sw, _) = tri(mode, true);
        let mut e = engine(&p);
        e.run_until(40 * SEC, BUDGET);
        lines(&mut e, sw[2], &["enable", "configure terminal", "interface gi1/0/2", "spanning-tree cost 50000", "end"]);
        e.run_until(e.now() + 60 * SEC, BUDGET);
        t.eq(&format!("{}: cost moves the root port", mode), e.stp_view(sw[2]).map(|v| (v.root_port, v.root_cost)), Some((Some(0), 40_000)));
        t.eq(&format!("{}: the other port blocks", mode), port_of(&e, sw[2], 1).map(|x| x.0), Some(PortRole::Alternate));
        t.check(&format!("{}: cost in the config", mode), e.running_config(sw[2]).contains("interface GigabitEthernet1/0/2\n spanning-tree cost 50000\n!"));
        // a better priority makes SW3 the root (the per-VLAN spelling is taken too)
        lines(&mut e, sw[2], &["configure terminal", "spanning-tree vlan 1 priority 0", "end"]);
        e.run_until(e.now() + 60 * SEC, BUDGET);
        t.check(&format!("{}: sw3 the root", mode), e.stp_view(sw[2]).is_some_and(|v| v.is_root()) && e.stp_view(sw[0]).is_some_and(|v| v.root == BridgeId::new(0, Mac([2, 0x4A, 0, 0, 3, 0]))));
        t.check(&format!("{}: written as the global form", mode), e.running_config(sw[2]).contains("spanning-tree mode ") && e.running_config(sw[2]).contains("spanning-tree priority 0\n"));
    }
    // two cables between SW1 and SW2: SW2's root port is the one at SW1's better port ID
    let mut p = Project::new("pair", 6);
    let a = p.add_device("jf-s24p", 0, 0).unwrap_or(0);
    let b = p.add_device("jf-s24p", 200, 0).unwrap_or(0);
    let _ = p.connect(&pr(a, "gi1/0/1"), &pr(b, "gi1/0/2"), None);
    let _ = p.connect(&pr(a, "gi1/0/2"), &pr(b, "gi1/0/1"), None);
    for (d, n) in [(a, "SW1"), (b, "SW2")] {
        if let Some(x) = p.device_mut(d) {
            x.startup_config = sw_cfg(n, "rstp", "");
        }
    }
    let mut e = engine(&p);
    e.run_until(5 * SEC, BUDGET);
    t.eq("root port at port 128.1", e.stp_view(b).map(|v| v.root_port), Some(Some(1)));
    lines(&mut e, a, &["enable", "configure terminal", "interface gi1/0/2", "spanning-tree port-priority 64", "end"]);
    e.run_until(e.now() + 5 * SEC, BUDGET);
    t.eq("port priority 64 wins", e.stp_view(b).map(|v| v.root_port), Some(Some(0)));
    t.check("port priority in the config", e.running_config(a).contains("interface GigabitEthernet1/0/2\n spanning-tree port-priority 64\n!"));
    cli(&mut e, a, "configure terminal");
    t.eq("multiple of 16", lines(&mut e, a, &["interface gi1/0/1", "spanning-tree port-priority 100"]), String::from("% Port Priority in increments of 16 is required\n"));
    t.eq("multiple of 4096", cli(&mut e, a, "spanning-tree priority 4097"), String::from("% Bridge Priority must be in increments of 4096.\n% Allowed are 0, 4096, 8192 ... 61440.\n"));
    t.eq("timers must fit", cli(&mut e, a, "spanning-tree forward-time 4"), String::from("% The timers do not fit together: 2 x (forward-time - 1) >= max-age >= 2 x (hello-time + 1)\n"));
    // 10G links cost 2000
    let mut p2 = Project::new("ten", 7);
    let x = p2.add_device("jf-s24p", 0, 0).unwrap_or(0);
    let y = p2.add_device("jf-s24p", 200, 0).unwrap_or(0);
    let _ = p2.connect(&pr(x, "te1/1/1"), &pr(y, "te1/1/1"), None);
    for (d, n) in [(x, "SW1"), (y, "SW2")] {
        if let Some(q) = p2.device_mut(d) {
            q.startup_config = sw_cfg(n, "stp", "");
        }
    }
    let e2 = engine(&p2);
    t.eq("10G costs 2000", e2.stp_view(y).and_then(|v| v.ports.first().map(|q| q.cost)), Some(2000));
}

// a hub with two ports of one switch: one is the designated port, the other a backup port; the
// link is shared; RSTP needs the timers there
fn hub(t: &mut T) {
    for mode in ["stp", "rstp"] {
        let mut p = Project::new("hub", 8);
        let s = p.add_device("jf-s24p", 0, 0).unwrap_or(0);
        let h = p.add_device("hub", 200, 0).unwrap_or(0);
        let _ = p.connect(&pr(s, "gi1/0/1"), &pr(h, "port0"), None);
        let _ = p.connect(&pr(s, "gi1/0/2"), &pr(h, "port1"), None);
        if let Some(d) = p.device_mut(s) {
            d.startup_config = sw_cfg("SW1", mode, "");
        }
        let mut e = engine(&p);
        e.run_until(40 * SEC, BUDGET);
        t.eq(&format!("{}: backup port", mode), (port_of(&e, s, 0).map(|x| x.0), port_of(&e, s, 1).map(|x| x.0)), (Some(PortRole::Designated), Some(PortRole::Backup)));
        t.check(&format!("{}: shared", mode), e.stp_view(s).is_some_and(|v| v.ports.iter().all(|x| !x.p2p)));
        t.check(&format!("{}: the hub repeats bpdus", mode), e.trace.hops().any(|q| q.from.0 == h && q.mask & decode::P_STP != 0));
        let fwd = state_times(&e, s, 0, PortState::Forwarding);
        // STP: 30 s; RSTP on a shared link: no agreement, fdWhile (Max Age 20 s) and then
        // twice Hello Time (forwardDelay while the port speaks RSTP)
        t.eq(&format!("{}: designated forwards", mode), fwd, alloc::vec![if mode == "stp" { 30 * SEC } else { 22 * SEC }]);
        t.check(&format!("{}: show says Shr and Back", mode), {
            let o = lines(&mut e, s, &["enable", "show spanning-tree"]);
            o.contains("Gi1/0/2             Back BLK 200000    128.2    Shr")
        });
    }
}

// the commands, the show output, the running configuration read back, debug
fn commands(t: &mut T) {
    let (p, sw, _) = tri("rapid-pvst", true);
    let mut e = engine(&p);
    e.run_until(10 * SEC, BUDGET);
    let o = lines(&mut e, sw[2], &["enable", "show spanning-tree"]);
    let want = "Spanning tree: IEEE 802.1w (RSTP), one tree for every VLAN (\"rapid-pvst\" runs as RSTP)\n  Root ID    Priority    4096\n             Address     024a.0000.0100\n             Cost        20000\n             Port        2 (GigabitEthernet1/0/2)\n             Hello Time   2 sec  Max Age 20 sec  Forward Delay 15 sec\n\n  Bridge ID  Priority    32768  (priority 32768 sys-id-ext 0)\n             Address     024a.0000.0300\n             Hello Time   2 sec  Max Age 20 sec  Forward Delay 15 sec\n             Aging Time  300 sec\n\nInterface           Role Sts Cost      Prio.Nbr Type\n------------------- ---- --- --------- -------- --------------------------------\nGi1/0/1             Altn BLK 20000     128.1    P2p\nGi1/0/2             Root FWD 20000     128.2    P2p\nGi1/0/3             Desg FWD 20000     128.3    P2p Edge\n";
    t.eq("show spanning-tree", o, String::from(want));
    let r = cli(&mut e, sw[2], "show spanning-tree root");
    t.check("show root", r.ends_with("all VLANs        4096 024a.0000.0100      20000     2  20  15  Gi1/0/2\n"));
    let i = cli(&mut e, sw[2], "show spanning-tree interface gi1/0/1");
    t.check("show interface", i.ends_with("all VLANs           Altn BLK 20000     128.1    P2p\n"));
    let d = cli(&mut e, sw[2], "show spanning-tree interface gi1/0/1 detail");
    t.check("show detail", d.starts_with("GigabitEthernet1/0/1 (port 128.1) is an alternate port, discarding\n") && d.contains("   Designated bridge 32768.024a.0000.0200, port 128.2\n"));
    let all = cli(&mut e, sw[2], "show spanning-tree detail");
    t.check("show all in detail", all.contains("  Root 4096.024a.0000.0100 at cost 20000 over GigabitEthernet1/0/2\n") && all.matches(" (port 128.").count() == 3);
    t.check("show vlan", cli(&mut e, sw[2], "show spanning-tree vlan 10").starts_with("Spanning tree: IEEE 802.1w (RSTP)"));
    t.check("bad vlan list", cli(&mut e, sw[2], "show spanning-tree vlan x").starts_with("% Bad VLAN list"));
    // help in our own words
    lines(&mut e, sw[2], &["configure terminal"]);
    let h = e.cli_help(sw[2], "spanning-tree mode ");
    t.check("help", h.contains("rapid-pvst") && h.contains("Taken for recognition: IEEE 802.1w, one tree for all VLANs") && h.contains("IEEE 802.1D, the classic spanning tree"));
    t.check("interface help", { lines(&mut e, sw[2], &["interface gi1/0/1"]); e.cli_help(sw[2], "spanning-tree ").contains("portfast") });
    lines(&mut e, sw[2], &["end"]);
    // the running configuration, read back as the startup configuration of a new engine
    let run = e.running_config(sw[0]);
    t.check("global lines", run.contains("hostname SW1\n!\nspanning-tree mode rapid-pvst\nspanning-tree priority 4096\n!\n"));
    t.check("interface lines", run.contains("interface GigabitEthernet1/0/3\n spanning-tree portfast\n!"));
    let mut p2 = p.clone();
    for &s in &sw {
        let r = e.running_config(s);
        if let Some(dv) = p2.device_mut(s) {
            dv.startup_config = r;
        }
    }
    let mut e2 = engine(&p2);
    e2.run_until(10 * SEC, BUDGET);
    let tree = |e: &Engine, s: u32| e.stp_view(s).map(|v| (v.bridge, v.root, v.root_port, v.ports.iter().map(|x| (x.port, x.id, x.role, x.state, x.cost, x.edge)).collect::<Vec<_>>()));
    t.check("round trip", sw.iter().all(|&s| tree(&e2, s) == tree(&e, s)) && sw.iter().all(|&s| e2.boot_errors(s).is_empty()));
    // debug spanning-tree events
    t.eq("debug on", cli(&mut e, sw[2], "debug spanning-tree events"), String::from("Debug output for spanning tree events: on\n"));
    t.eq("show debugging", cli(&mut e, sw[2], "show debugging"), String::from("Debug output for spanning tree events: on\n"));
    let _ = e.take_reports();
    unplug(&mut e, &p, sw[0], "gi1/0/2");
    e.run_until(e.now() + 10 * MS, BUDGET);
    let dbg = out(&mut e, sw[2]);
    t.check("debug lines", dbg.contains(": STP: Gi1/0/1 is now root (was alternate)\n") && dbg.contains(": STP: Gi1/0/1 goes from discarding to learning\n"));
    t.eq("undebug", cli(&mut e, sw[2], "undebug spanning-tree events"), String::from("Debug output for spanning tree events: off\n"));
    t.eq("debug all", cli(&mut e, sw[2], "debug all"), String::from("Debug output: all on (a busy network prints a lot)\n"));
    t.check("debug all includes it", cli(&mut e, sw[2], "show debugging").contains("Debug output for spanning tree events: on\n"));
    cli(&mut e, sw[2], "undebug all");
    // spanning tree off again: everything forwards as before
    lines(&mut e, sw[1], &["enable", "configure terminal", "no spanning-tree mode", "end"]);
    t.check("off again", e.stp_view(sw[1]).is_none() && e.view().links.iter().filter(|l| l.ends.iter().any(|x| x.0 == sw[1])).all(|l| l.led[if l.ends[0].0 == sw[1] { 0 } else { 1 }] == Led::Up));
    t.check("mode line gone", !e.running_config(sw[1]).contains("spanning-tree mode"));
    // a routed port of an L3 switch takes no spanning tree settings
    let mut p3 = Project::new("l3", 9);
    let m = p3.add_device("jf-s24l3", 0, 0).unwrap_or(0);
    if let Some(dv) = p3.device_mut(m) {
        dv.startup_config = String::from("hostname MLS1\n!\nspanning-tree mode rstp\n!\ninterface GigabitEthernet1/0/1\n no switchport\n!\nend\n");
    }
    let mut e3 = engine(&p3);
    let o3 = lines(&mut e3, m, &["enable", "configure terminal", "interface gi1/0/1", "spanning-tree portfast"]);
    t.check("routed port refuses", o3.contains("% Invalid input detected at '^' marker."));
    // an access point has no spanning tree
    let mut p4 = Project::new("ap", 10);
    let ap = p4.add_device("jf-ap6e", 0, 0).unwrap_or(0);
    let mut e4 = engine(&p4);
    t.check("no command on an access point", lines(&mut e4, ap, &["enable", "configure terminal", "spanning-tree mode rstp"]).contains('^'));
}

// LEDs show the port states; BPDUs are frames of the simulation mode like any other
fn leds_and_sim_mode(t: &mut T) {
    let (p, sw, _) = tri("stp", true);
    let mut e = engine(&p);
    e.run_until(SEC / 2, BUDGET);
    t.eq("listening is amber", e.port_led(sw[1], 0), Led::Blocking);
    t.eq("edge port green", e.port_led(sw[1], 2), Led::Up);
    e.run_until(16 * SEC, BUDGET);
    t.eq("learning", e.port_led(sw[1], 0), Led::Learning);
    e.run_until(31 * SEC, BUDGET);
    t.eq("forwarding", e.port_led(sw[1], 0), Led::Up);
    t.eq("alternate amber", e.port_led(sw[2], 0), Led::Blocking);
    let lv = e.view().links.into_iter().find(|l| l.ends == [(sw[1], 1), (sw[2], 0)]);
    t.eq("the cable's leds", lv.map(|l| l.led), Some([Led::Up, Led::Blocking]));
    // simulation mode: STP frames on the cables, rows of the event list
    let f = jfn_sim::Filter::protocols(decode::P_STP);
    t.check("filter passes bpdus", e.trace.hops().filter(|h| f.hop(h)).count() > 20);
    t.check("frames at a moment", !e.frames_at(30 * SEC + 300, &f, 1000).is_empty());
    let ev = e.trace.events().find(|x| x.dev == sw[1] && matches!(x.note, Note::StpState { .. })).cloned();
    let row = ev.as_ref().map(|x| e.event_row(x));
    t.eq("event row", row.map(|r| (r.proto, r.layer, r.text)), Some(("STP", 2, String::from("Spanning Tree: GigabitEthernet1/0/1 wechselt von Blocking nach Listening."))));
    let rx = e.trace.events().find(|x| x.dev == sw[1] && matches!(x.note, Note::StpRx { .. })).cloned();
    t.check("rx row", rx.is_some_and(|x| {
        let r = e.event_row(&x);
        r.proto == "STP" && r.text.starts_with("BPDU an GigabitEthernet1/0/1: Configuration BPDU von 4096.024a.0000.0100, Root 4096.024a.0000.0100, Kosten 0")
    }));
    // PC2's ARP request floods over SW2 to SW3, whose alternate port takes nothing in
    let (p2, sw2, pcs2) = tri("stp", true);
    let mut e2 = engine(&p2);
    e2.run_until(31 * SEC, BUDGET);
    t.eq("ping over the tree", ping(&mut e2, pcs2[1], "10.0.0.1", SEC), String::from(OK1));
    let blocked: Vec<Note> = notes(&e2, sw2[2]).into_iter().filter(|(_, n)| matches!(n, Note::StpBlocked { .. })).map(|x| x.1).collect();
    t.check("the alternate port drops", !blocked.is_empty() && blocked.iter().all(|n| *n == Note::StpBlocked { port: 0, state: PortState::Blocking } && n.is_drop()));
}

// a loop is a problem unless every bridge in it runs spanning tree
fn problems(t: &mut T) {
    let loops = |e: &Engine| -> Vec<String> { e.problems().into_iter().filter(|p| p.kind == ProblemKind::Loop).map(|p| p.text).collect() };
    let (p, _, _) = tri("rstp", true);
    let e = engine(&p);
    t.eq("all with spanning tree", loops(&e), Vec::<String>::new());
    let (p2, _, _) = triangle([sw_cfg("SW1", "rstp", ""), sw_cfg("SW2", "stp", ""), sw_cfg("SW3", "", "")]);
    let e2 = engine(&p2);
    t.eq("one without", loops(&e2), alloc::vec![String::from("Schleife über SW1, SW2, SW3 (VLAN 1): SW3 ohne Spanning Tree, Broadcasts kreisen endlos; „spanning-tree mode rstp“ dort schaltet ihn ein.")]);
}

// the same project twice gives the same run
fn determinism(t: &mut T) {
    for mode in ["stp", "rstp"] {
        let (p, sw, _) = tri(mode, false);
        let run = || {
            let mut e = engine(&p);
            e.run_until(45 * SEC, BUDGET);
            unplug(&mut e, &p, sw[0], "gi1/0/1");
            e.run_until(100 * SEC, BUDGET);
            let hops: Vec<(Ns, Ns, Vec<u8>)> = e.trace.hops().map(|h| (h.t_tx, h.t_rx, h.bytes.to_vec())).collect();
            let evs: Vec<(Ns, u32, Note)> = e.trace.events().map(|x| (x.t, x.dev, x.note.clone())).collect();
            (hops, evs, sw.iter().map(|&s| e.stp_view(s)).collect::<Vec<_>>())
        };
        let (a, b) = (run(), run());
        t.check(&format!("{}: same frames", mode), a.0 == b.0 && !a.0.is_empty());
        t.check(&format!("{}: same decisions", mode), a.1 == b.1);
        t.check(&format!("{}: same trees", mode), a.2 == b.2);
    }
}

// the new notes have their English
fn english(t: &mut T) {
    let rows: Vec<(&str, &str)> = include_str!("locales/engine.tsv").lines().skip(1).filter(|l| !l.is_empty() && !l.starts_with('#')).filter_map(|l| l.split_once('\t')).collect();
    let en = move |k: &'static str| rows.iter().find(|r| r.0 == k).map_or(k, |r| r.1);
    let pn = |p: u16| format!("Gi1/0/{}", p + 1);
    let iface = |i: u16| format!("if{}", i);
    let b = Bpdu::rst(0, BridgeId::new(4096, Mac([2, 0x4A, 0, 0, 1, 0])), 0, BridgeId::new(4096, Mac([2, 0x4A, 0, 0, 1, 0])), PortId::new(128, 1), Times { msg_age: 0, max_age: 20, hello: 2, fwd_delay: 15 }).with_role(Role::Designated);
    let ns = [
        Note::StpRx { port: 0, bpdu: b },
        Note::StpTx { port: 0, bpdu: Bpdu::tcn() },
        Note::StpDropped { port: 0, why: jfn_sim::stp::BpduDrop::Type },
        Note::StpBlocked { port: 1, state: PortState::Discarding },
        Note::StpRole { port: 1, from: PortRole::Designated, to: PortRole::Alternate },
        Note::StpState { port: 1, from: PortState::Learning, to: PortState::Forwarding },
        Note::StpRoot { root: b.root, port: Some(0), cost: 20_000 },
        Note::StpTopology { port: Some(0), what: jfn_sim::stp::TcWhat::TcnReceived },
        Note::StpFlush { port: 0, entries: 3 },
        Note::StpAging { secs: 15, fast: true },
        Note::StpAged { port: 0 },
        Note::StpEdge { port: 2, edge: true },
        Note::StpMigrate { port: 0, rstp: false },
        Note::StpErrDisable { port: 2 },
    ];
    let texts: Vec<String> = ns.iter().map(|n| n.message(&pn, &iface).text_with(&en)).collect();
    t.check("every note in English", ns.iter().zip(texts.iter()).all(|(n, e)| *e != n.message(&pn, &iface).text()));
    t.eq("an english role change", texts[4].clone(), String::from("Spanning tree: Gi1/0/2 is now alternate port (was designated port)."));
    t.eq("an english state change", texts[5].clone(), String::from("Spanning tree: Gi1/0/2 goes from Learning to Forwarding."));
}
