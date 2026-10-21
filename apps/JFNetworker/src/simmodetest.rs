// simmodetest -- checks of the simulation mode of jfn_sim (part of --selftest): what the
// recording keeps (causes, batches, numbers), filters, steps forward and back along the
// recording, the frames on the cables at a moment, drops, the event list, the packet tab, HDLC
// frames in the inspector, pcapng files (block by block), the problems list and the thread
// driver of the app (src/sim_thread.rs)
//
// `JFNetworker --selftest --pcap-out DIR` also writes the capture files of these checks into
// DIR (e7-*.pcapng) for Wireshark or tshark.
//
// Times as in l3test.rs: 1 Gbit/s copper with 50 ns of cable; an ARP frame takes 722 ns per
// cable, a 98-byte echo frame 1026 ns.

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use jfn_model::project::{PortRef, Project};
use jfn_model::CableKind;
use jfn_sim::node::port_mac;
use jfn_sim::view::{hop_info, hop_layers, proto_name};
use jfn_sim::{Action, Cmd, DevStep, DropMark, Engine, Filter, HopDrop, InFlight, Inline, Note, PcapOpts, PcapScope, PingOpts, ProblemKind, Severity, TraceOpts, VlanSet, MS, SEC};
use netpkt::decode::{self, Proto, P_ARP, P_DNS, P_ETH, P_ICMP, P_IPV4, P_UDP};
use netpkt::pcapng;
use netpkt::{Ipv4, Prefix4};

use crate::selftest::T;

const BUDGET: usize = 1_000_000;

fn pr(dev: u32, port: &str) -> PortRef {
    PortRef { dev, port: String::from(port) }
}

fn ip(s: &str) -> Ipv4 {
    Ipv4::parse(s).unwrap_or(Ipv4::UNSPECIFIED)
}

fn pf(s: &str) -> Prefix4 {
    Prefix4::parse(s).unwrap_or_default()
}

fn engine(p: &Project) -> Engine {
    Engine::from_project(p).unwrap_or_else(|_| Engine::new(0))
}

fn addr(e: &mut Engine, dev: u32, iface: &str, p: &str) -> bool {
    e.apply(Action::IfAddr { dev, iface: String::from(iface), prefix: pf(p), add: true }).is_ok()
}

fn up(e: &mut Engine, dev: u32, iface: &str) -> bool {
    e.apply(Action::IfUp { dev, iface: String::from(iface), up: true }).is_ok()
}

fn gw(e: &mut Engine, dev: u32, via: &str, add: bool) -> bool {
    e.apply(Action::Route { dev, prefix: pf("0.0.0.0/0"), via: Some(ip(via)), iface: None, ad: 1, add }).is_ok()
}

fn once() -> PingOpts {
    PingOpts { count: 1, ..PingOpts::default() }
}

fn ping(e: &mut Engine, dev: u32, dst: &str, o: PingOpts) {
    let _ = e.ping(dev, ip(dst), o);
}

/// PC1 .. PC3 on SW1 Gi1/0/1 .. 3 (ids 1 .. 3 and 4, cables 5 .. 7), without addresses
fn lan3_project() -> (Project, [u32; 3], u32) {
    let mut p = Project::new("lan3", 21);
    let pcs = [0, 1, 2].map(|i| p.add_device("pc", i * 100, 0).unwrap_or(0));
    let sw = p.add_device("jf-s24p", 100, 100).unwrap_or(0);
    for (i, &pc) in pcs.iter().enumerate() {
        let _ = p.connect(&pr(pc, "eth0"), &pr(sw, &format!("gi1/0/{}", i + 1)), None);
    }
    (p, pcs, sw)
}

/// lan3 with 10.0.0.1 .. 3/24: (engine, pcs, switch, set-up ok)
fn lan3() -> (Engine, [u32; 3], u32, bool) {
    let (p, pcs, sw) = lan3_project();
    let mut e = engine(&p);
    let ok = [addr(&mut e, pcs[0], "eth0", "10.0.0.1/24"), addr(&mut e, pcs[1], "eth0", "10.0.0.2/24"), addr(&mut e, pcs[2], "eth0", "10.0.0.3/24")];
    (e, pcs, sw, ok.iter().all(|&b| b))
}

/// lan3 after PC1 pinged PC2 once (nine frames, twenty decisions)
fn lan3_ping() -> (Engine, [u32; 3], u32, bool) {
    let (mut e, pcs, sw, ok) = lan3();
    ping(&mut e, pcs[0], "10.0.0.2", once());
    e.run_idle(BUDGET);
    (e, pcs, sw, ok)
}

/// PC1 (10.0.1.10) - Gi0/0/0 R1 Gi0/0/1 - PC2 (10.0.2.10) with default routes; the router's
/// interfaces brought up when `on`: (engine, pc1, r1, pc2, set-up ok)
fn routed(on: bool) -> (Engine, u32, u32, u32, bool) {
    let mut p = Project::new("routed", 22);
    let pc1 = p.add_device("pc", 0, 0).unwrap_or(0);
    let r1 = p.add_device("jf-r4", 100, 0).unwrap_or(0);
    let pc2 = p.add_device("pc", 200, 0).unwrap_or(0);
    let _ = p.connect(&pr(pc1, "eth0"), &pr(r1, "gi0/0/0"), None);
    let _ = p.connect(&pr(r1, "gi0/0/1"), &pr(pc2, "eth0"), None);
    let mut e = engine(&p);
    let mut ok = addr(&mut e, pc1, "eth0", "10.0.1.10/24")
        && gw(&mut e, pc1, "10.0.1.1", true)
        && addr(&mut e, pc2, "eth0", "10.0.2.10/24")
        && gw(&mut e, pc2, "10.0.2.1", true)
        && addr(&mut e, r1, "gi0/0/0", "10.0.1.1/24")
        && addr(&mut e, r1, "gi0/0/1", "10.0.2.1/24");
    if on {
        ok = ok && up(&mut e, r1, "gi0/0/0") && up(&mut e, r1, "gi0/0/1");
    }
    (e, pc1, r1, pc2, ok)
}

/// R1 Se0/2/0 (10.0.12.1/30) - R2 Se0/2/0 (10.0.12.2/30): (engine, r1, r2, set-up ok)
fn serial() -> (Engine, u32, u32, bool) {
    let mut p = Project::new("wan", 23);
    let r1 = p.add_device("jf-r4", 0, 0).unwrap_or(0);
    let r2 = p.add_device("jf-r4", 200, 0).unwrap_or(0);
    let _ = p.connect(&pr(r1, "se0/2/0"), &pr(r2, "se0/2/0"), None);
    let mut e = engine(&p);
    let ok = addr(&mut e, r1, "se0/2/0", "10.0.12.1/30") && addr(&mut e, r2, "se0/2/0", "10.0.12.2/30") && up(&mut e, r1, "se0/2/0") && up(&mut e, r2, "se0/2/0");
    (e, r1, r2, ok)
}

/// router on a stick: PC1 (VLAN 10) and PC2 (VLAN 20) on SW1, SW1 Gi1/0/24 to R1 Gi0/0/0 with
/// subinterfaces; the switch port is a trunk when `trunk`: (engine, pc1, sw, r1, set-up ok)
fn stick(trunk: bool) -> (Engine, u32, u32, u32, bool) {
    let mut p = Project::new("stick", 24);
    let pc1 = p.add_device("pc", 0, 0).unwrap_or(0);
    let pc2 = p.add_device("pc", 200, 0).unwrap_or(0);
    let sw = p.add_device("jf-s24p", 100, 100).unwrap_or(0);
    let r1 = p.add_device("jf-r4", 100, 200).unwrap_or(0);
    let _ = p.connect(&pr(pc1, "eth0"), &pr(sw, "gi1/0/1"), None);
    let _ = p.connect(&pr(pc2, "eth0"), &pr(sw, "gi1/0/2"), None);
    let _ = p.connect(&pr(sw, "gi1/0/24"), &pr(r1, "gi0/0/0"), None);
    let mut e = engine(&p);
    let mut ok = [
        e.apply(Action::Access { dev: sw, port: String::from("gi1/0/1"), vlan: 10 }).is_ok(),
        e.apply(Action::Access { dev: sw, port: String::from("gi1/0/2"), vlan: 20 }).is_ok(),
        up(&mut e, r1, "gi0/0/0"),
        e.apply(Action::SubIf { dev: r1, name: String::from("gi0/0/0.10"), vlan: 10, add: true }).is_ok(),
        e.apply(Action::SubIf { dev: r1, name: String::from("gi0/0/0.20"), vlan: 20, add: true }).is_ok(),
        addr(&mut e, r1, "gi0/0/0.10", "10.0.10.1/24"),
        addr(&mut e, r1, "gi0/0/0.20", "10.0.20.1/24"),
        addr(&mut e, pc1, "eth0", "10.0.10.10/24"),
        gw(&mut e, pc1, "10.0.10.1", true),
        addr(&mut e, pc2, "eth0", "10.0.20.10/24"),
        gw(&mut e, pc2, "10.0.20.1", true),
    ]
    .iter()
    .all(|&b| b);
    if trunk {
        ok = ok && e.apply(Action::Trunk { dev: sw, port: String::from("gi1/0/24"), native: 1, allowed: VlanSet::all() }).is_ok();
    }
    (e, pc1, sw, r1, ok)
}

/// with --pcap-out DIR: writes a capture file into DIR
fn save(name: &str, bytes: &[u8]) {
    let Some(dir) = plat::arg_value("--pcap-out") else {
        return;
    };
    let path = format!("{}/{}", dir, name);
    let ok = plat::write_file(&path, bytes);
    plat::write_stdout(format!("pcap {} {}\n", if ok { "written:" } else { "NOT written:" }, path).as_bytes());
}

pub fn run(t: &mut T) {
    recording(t);
    filters(t);
    stepping(t);
    animation(t);
    event_list(t);
    packet_tab(t);
    hdlc(t);
    capture(t);
    capture_files(t);
    problems_ip(t);
    problems_l2(t);
    #[cfg(any(windows, target_os = "linux"))]
    threaded(t);
}

fn recording(t: &mut T) {
    let (e, pcs, sw, ok) = lan3_ping();
    let [pc1, pc2, pc3] = pcs;
    t.check("lan set-up", ok);
    let hops: Vec<(u64, u32, u32, u64, u64, Option<u64>)> = e.trace.hops().map(|h| (h.id, h.from.0, h.to.0, h.t_tx, h.t_rx, h.cause)).collect();
    let want = vec![
        (1, pc1, sw, 0, 722, None),
        (2, sw, pc2, 722, 1444, Some(1)),
        (3, sw, pc3, 722, 1444, Some(1)),
        (4, pc2, sw, 1444, 2166, Some(2)),
        (5, sw, pc1, 2166, 2888, Some(4)),
        (6, pc1, sw, 2888, 3914, None),
        (7, sw, pc2, 3914, 4940, Some(6)),
        (8, pc2, sw, 4940, 5966, Some(7)),
        (9, sw, pc1, 5966, 6992, Some(8)),
    ];
    t.eq("frames and their causes", hops, want);
    t.check("nothing waited", e.trace.hops().all(|h| h.t_queued == h.t_tx && !h.hdlc));
    let notes: Vec<(u64, u32, Note)> = e.trace.events().skip(6).map(|x| (x.seq, x.dev, x.note.clone())).collect();
    let (m1, m2) = (port_mac(pc1, 0), port_mac(pc2, 0));
    let want = vec![
        (7, pc1, Note::ArpRequest { iface: 1, ip: ip("10.0.0.2") }),
        (8, sw, Note::Learned { mac: m1, port: 0, vlan: 1 }),
        (9, sw, Note::Flooded { vlan: 1, ports: 2 }),
        (10, pc2, Note::ArpLearned { ip: ip("10.0.0.1"), mac: m1 }),
        (11, pc2, Note::ArpReply { iface: 1, ip: ip("10.0.0.2"), mac: m2 }),
        (12, pc3, Note::ArpIgnored { ip: ip("10.0.0.2") }),
        (13, sw, Note::Learned { mac: m2, port: 1, vlan: 1 }),
        (14, sw, Note::Forwarded { vlan: 1, port: 0 }),
        (15, pc1, Note::ArpLearned { ip: ip("10.0.0.2"), mac: m2 }),
        (16, sw, Note::Forwarded { vlan: 1, port: 1 }),
        (17, pc2, Note::IpLocal { proto: 1 }),
        (18, pc2, Note::EchoAnswered { to: ip("10.0.0.1") }),
        (19, sw, Note::Forwarded { vlan: 1, port: 0 }),
        (20, pc1, Note::IpLocal { proto: 1 }),
    ];
    t.eq("decisions in order", notes, want);
    t.eq("numbers", (e.trace.events().next().map(|x| x.seq), e.trace.next_event_seq()), (Some(1), 21));
    t.check("decision by number", e.trace.event(12).is_some_and(|x| x.dev == pc3 && x.hop == Some(3)) && e.trace.event(0).is_none() && e.trace.event(21).is_none());
    t.eq("decisions after", e.trace.events_after(18).map(|x| x.seq).collect::<Vec<_>>(), vec![19, 20]);
    t.eq("all decisions after 0", e.trace.events_after(0).count(), 20);
    t.eq("ignored", e.trace.event(12).map(|x| e.describe(x)), Some(String::from("ARP-Anfrage nach 10.0.0.2 ist nicht für dieses Gerät: verworfen.")));
    t.check("ignoring drops", Note::ArpIgnored { ip: ip("10.0.0.2") }.is_drop());
    // batches: the frames flooded together share one, the ping and the cabling do not
    let batch = |seq: u64| e.trace.event(seq).map(|x| x.batch).unwrap_or(0);
    let hb = |id: u64| e.trace.hop(id).map(|h| h.batch).unwrap_or(0);
    t.check("flooded in one batch", hb(2) == hb(3) && hb(2) == batch(9) && hb(1) != hb(2));
    t.check("the ping is a batch of its own", batch(7) != batch(1) && hb(1) == batch(7));
    t.check("the queued echo leaves with the answer", hb(6) == batch(15));
    t.eq("batch of the cabling", e.trace.batch_events(batch(1), sw).map(|x| x.seq).collect::<Vec<_>>(), vec![2, 4, 6]);
    t.eq("one device, one batch", e.trace.batch_events(batch(13), sw).map(|x| x.seq).collect::<Vec<_>>(), vec![13, 14]);
    t.eq("events between", e.trace.events_between(722, 1444).map(|x| x.seq).collect::<Vec<_>>(), vec![10, 11, 12]);
    t.eq("last event at", (e.trace.last_event_at(2166), e.trace.last_event_at(2165), e.trace.last_event_at(0)), (Some(14), Some(12), Some(7)));
}

fn filters(t: &mut T) {
    let (e, pcs, _sw, _) = lan3_ping();
    let [_pc1, pc2, pc3] = pcs;
    let pass = |f: &Filter| e.trace.hops().filter(|h| f.hop(h)).map(|h| h.id).collect::<Vec<_>>();
    t.eq("all frames", pass(&Filter::all()).len(), 9);
    t.eq("icmp frames", pass(&Filter::protocols(P_ICMP)), vec![6, 7, 8, 9]);
    t.eq("frames of a device", pass(&Filter { mask: 0, devices: vec![pc3] }), vec![3]);
    t.eq("arp frames of a device", pass(&Filter { mask: P_ARP, devices: vec![pc2] }), vec![2, 4]);
    let events = |f: &Filter| e.trace.events().filter(|x| f.event(&e.trace, x)).map(|x| x.seq).collect::<Vec<_>>();
    t.eq("icmp decisions", events(&Filter::protocols(P_ICMP)), vec![16, 17, 18, 19, 20]);
    t.eq("arp decisions", events(&Filter::protocols(P_ARP)), (7..=15).collect::<Vec<u64>>());
    t.eq("decisions of a device", events(&Filter { mask: 0, devices: vec![pc3] }), vec![5, 12]);
    t.eq("own protocols", [Note::ArpRequest { iface: 1, ip: ip("10.0.0.2") }.mask(), Note::IcmpSent { typ: 3, code: 3, to: ip("10.0.0.1") }.mask(), Note::LinkUp { port: 0 }.mask(), Note::IpLocal { proto: 17 }.mask()], [P_ARP, P_IPV4 | P_ICMP, 0, P_IPV4 | P_UDP]);
    t.eq("protocol names", [proto_name(P_ETH | P_IPV4 | P_UDP | P_DNS), proto_name(P_ETH | P_ARP), proto_name(P_ETH), proto_name(0)], ["DNS", "ARP", "Ethernet", ""]);
}

fn stepping(t: &mut T) {
    let sim = || {
        let (e, pcs, sw, _) = lan3();
        let mut d = Inline::new(e);
        let _ = d.send(Cmd::Simulation, 0);
        let _ = d.eng.ping(pcs[0], ip("10.0.0.2"), once());
        (d, pcs, sw)
    };
    let step = |d: &mut Inline, c: Cmd| d.send(c, 0).map(|x| (x.now, x.view)).unwrap_or((u64::MAX, None));
    // from one frame sent to the next
    let (mut d, pcs, _sw) = sim();
    let times: Vec<u64> = (0..7).map(|_| step(&mut d, Cmd::Step(Filter::all())).0).collect();
    t.eq("steps from frame to frame", times, vec![722, 1444, 2166, 2888, 3914, 4940, 5966]);
    let r = d.send(Cmd::Step(Filter::all()), 0);
    t.check("the last step runs out", r.is_ok_and(|x| !x.new_hops && x.now == 2 * SEC));
    // only some protocols or devices
    let (mut d, _, _) = sim();
    t.eq("to the first echo", step(&mut d, Cmd::Step(Filter::protocols(P_ICMP))), (2888, None));
    t.eq("then the next", step(&mut d, Cmd::Step(Filter::protocols(P_ICMP))), (3914, None));
    let (mut d, pcs3, _) = sim();
    t.eq("frames of PC3", step(&mut d, Cmd::Step(Filter { mask: 0, devices: vec![pcs3[2]] })), (722, None));
    t.eq("no more for PC3", step(&mut d, Cmd::Step(Filter { mask: 0, devices: vec![pcs3[2]] })).0, 2 * SEC);
    let (mut d, _, _) = sim();
    t.eq("step hop by mask", step(&mut d, Cmd::StepHop { mask: P_ARP }), (722, None));
    // back along the recording: only the view moves
    let (mut d, _, _) = sim();
    t.eq("nothing before the first frame", step(&mut d, Cmd::StepBack(Filter::all())), (0, None));
    for _ in 0..5 {
        let _ = d.send(Cmd::Step(Filter::all()), 0);
    }
    let events = d.eng.events_done();
    t.eq("back one", step(&mut d, Cmd::StepBack(Filter::all())), (3914, Some(2888)));
    t.eq("view time", (d.view(), d.view_time(), d.eng.now()), (Some(2888), 2888, 3914));
    t.eq("back two", step(&mut d, Cmd::StepBack(Filter::all())), (3914, Some(2166)));
    t.eq("no echo before", step(&mut d, Cmd::StepBack(Filter::protocols(P_ICMP))), (3914, Some(2166)));
    t.check("devices keep their state", d.eng.arp_table(pcs[0]).iter().any(|r| r.ip == ip("10.0.0.2") && r.mac.is_some()));
    t.eq("forward through the recording", step(&mut d, Cmd::Step(Filter::all())), (3914, Some(2888)));
    t.eq("up to the present", step(&mut d, Cmd::Step(Filter::all())), (3914, None));
    t.eq("the engine did not run", d.eng.events_done(), events);
    t.eq("then it runs again", step(&mut d, Cmd::Step(Filter::all())), (4940, None));
    let _ = d.send(Cmd::StepBack(Filter::all()), 0);
    let _ = d.send(Cmd::StepBack(Filter::all()), 0);
    t.eq("live again", step(&mut d, Cmd::Live), (4940, None));
    let _ = d.send(Cmd::StepBack(Filter::all()), 0);
    t.eq("running goes live", step(&mut d, Cmd::RunIdle), (2 * SEC, None));
    let _ = d.send(Cmd::StepBack(Filter::protocols(P_ARP)), 0);
    t.eq("back by protocol", d.view(), Some(2166));
    let r = d.send(Cmd::Realtime { speed_pm: 1000 }, 0);
    t.check("realtime goes live", r.is_ok_and(|x| x.view.is_none()));
}

fn animation(t: &mut T) {
    let (e, pcs, sw, _) = lan3_ping();
    let [pc1, pc2, pc3] = pcs;
    let all = Filter::all();
    let arp = P_ETH | P_ARP;
    let one = InFlight { hop: 1, link: 5, dir: 0, from: (pc1, 0), to: (sw, 0), t_tx: 0, t_rx: 722, pos: 500_000, mask: arp, drop: None };
    t.eq("half way", e.frames_at(361, &all, 100), vec![one]);
    t.check("as a fraction", e.frames_at(361, &all, 100).first().is_some_and(|f| f.fraction() == 0.5));
    let two = InFlight { hop: 2, link: 6, dir: 1, from: (sw, 1), to: (pc2, 0), t_tx: 722, t_rx: 1444, pos: 385_041, mask: arp, drop: None };
    let three = InFlight { hop: 3, link: 7, dir: 1, from: (sw, 2), to: (pc3, 0), t_tx: 722, t_rx: 1444, pos: 385_041, mask: arp, drop: None };
    t.eq("flooded frames", e.frames_at(1000, &all, 100), vec![two, three]);
    t.eq("the latest only", e.frames_at(1000, &all, 1).iter().map(|f| f.hop).collect::<Vec<_>>(), vec![3]);
    t.eq("just sent", e.frames_at(722, &all, 100).iter().map(|f| (f.hop, f.pos)).collect::<Vec<_>>(), vec![(2, 0), (3, 0)]);
    t.eq("arrived ones are gone", e.frames_at(1444, &all, 100).iter().map(|f| (f.hop, f.pos)).collect::<Vec<_>>(), vec![(4, 0)]);
    t.eq("filtered", e.frames_at(3000, &Filter::protocols(P_ICMP), 100).iter().map(|f| (f.hop, f.pos)).collect::<Vec<_>>(), vec![(6, 109_161)]);
    t.eq("by device", e.frames_at(1000, &Filter { mask: 0, devices: vec![pc3] }, 100).iter().map(|f| f.hop).collect::<Vec<_>>(), vec![3]);
    t.eq("nothing at the end", e.frames_at(7000, &all, 100), Vec::new());
    // drops: PC3 throws the question away
    let mark = DropMark { t: 1444, dev: pc3, hop: Some(3), seq: Some(12), lost: None };
    t.eq("drops", e.drops_between(0, 10_000, &all), vec![mark]);
    t.eq("drops after", e.drops_between(1444, 10_000, &all), Vec::new());
    t.eq("drops filtered", e.drops_between(0, 10_000, &Filter::protocols(P_ICMP)), Vec::new());
    // a cable that loses everything
    let (mut e, pcs, sw, _) = lan3();
    let _ = e.apply(Action::Impair { link: 5, kbps: None, delay_ns: None, jitter_ns: None, loss_ppm: Some(1_000_000) });
    ping(&mut e, pcs[0], "10.0.0.2", once());
    e.run_idle(BUDGET);
    t.check("lost on the way", e.frames_at(361, &all, 100).first().is_some_and(|f| f.drop == Some(HopDrop::Loss)));
    let lost = DropMark { t: 722, dev: sw, hop: Some(1), seq: None, lost: Some(HopDrop::Loss) };
    t.eq("lost frame", e.drops_between(0, 722, &all), vec![lost]);
    t.check("then ARP gives up", e.drops_between(722, 10 * SEC, &all).iter().any(|d| d.dev == pcs[0] && d.t == 3 * SEC && d.hop.is_none()));
}

fn event_list(t: &mut T) {
    let (e, pcs, sw, _) = lan3_ping();
    let [pc1, pc2, pc3] = pcs;
    let rows = e.event_rows(&Filter::all(), 0, 1000);
    t.eq("rows", rows.len(), 20);
    let row = |seq: u64| rows.iter().find(|r| r.seq == seq).cloned();
    t.eq(
        "row of a drop",
        row(12).map(|r| (r.t, r.from, r.dev, r.hop, r.layer, r.proto, r.mask, r.text, r.drop)),
        Some((1444, Some(sw), pc3, Some(3), 2, "ARP", P_ETH | P_ARP, String::from("ARP-Anfrage nach 10.0.0.2 ist nicht für dieses Gerät: verworfen."), true)),
    );
    t.eq("row of a device on its own", row(7).map(|r| (r.from, r.hop, r.proto, r.text)), Some((None, None, "ARP", String::from("ARP-Anfrage an eth0: Wer hat 10.0.0.2?"))));
    t.eq("row of the echo", row(18).map(|r| (r.from, r.dev, r.layer, r.proto, r.drop)), Some((Some(sw), pc2, 3, "ICMP", false)));
    t.eq("row of a cable", row(1).map(|r| (r.dev, r.layer, r.proto, r.text)), Some((pc1, 1, "", String::from("Leitung an eth0 ist aktiv."))));
    let seqs = |f: &Filter, after: u64, limit: usize| e.event_rows(f, after, limit).iter().map(|r| r.seq).collect::<Vec<_>>();
    t.eq("icmp rows", seqs(&Filter::protocols(P_ICMP), 0, 100), vec![16, 17, 18, 19, 20]);
    t.eq("rows of PC3", seqs(&Filter { mask: 0, devices: vec![pc3] }, 0, 100), vec![5, 12]);
    t.eq("rows after", seqs(&Filter::all(), 15, 3), vec![16, 17, 18]);
    // a router asks for the next hop of a ping: its own protocol, the frame's bits
    let (mut e, pc1, r1, _pc2, _) = routed(true);
    ping(&mut e, pc1, "10.0.2.10", once());
    e.run_idle(BUDGET);
    let rows = e.event_rows(&Filter { mask: 0, devices: vec![r1] }, 0, 1000);
    let ask = rows.iter().find(|r| r.dev == r1 && r.text.starts_with("ARP-Anfrage an GigabitEthernet0/0/1"));
    t.check("the router asks for PC2", ask.is_some_and(|r| r.proto == "ARP" && r.mask & P_ICMP != 0 && r.from == Some(pc1)));
    let routed_row = rows.iter().find(|r| r.dev == r1 && r.text.starts_with("Route zu 10.0.2.10"));
    t.check("routing a ping", routed_row.is_some_and(|r| r.proto == "ICMP" && r.layer == 3));
    t.check("ios drops while asking", rows.iter().any(|r| r.dev == r1 && r.drop && r.proto == "ARP" && r.text.contains("noch unbekannt")));
}

/// (inbound, decisions, outbound) of a step
fn parts(s: &DevStep) -> (Option<u64>, Vec<u64>, Vec<u64>) {
    (s.inbound, s.decisions.iter().map(|x| x.seq).collect(), s.outbound.clone())
}

fn packet_tab(t: &mut T) {
    let (mut e, pcs, sw, _) = lan3_ping();
    let ev = |e: &Engine, seq: u64| e.trace.event(seq).cloned();
    let step = |e: &Engine, seq: u64| ev(e, seq).map(|x| parts(&e.step_of(&x)));
    t.eq("switch forwards", step(&e, 14), Some((Some(4), vec![13, 14], vec![5])));
    t.check("step time and device", ev(&e, 14).is_some_and(|x| e.step_of(&x).t == 2166 && e.step_of(&x).dev == sw));
    t.eq("answer frees the queued echo", step(&e, 15), Some((Some(5), vec![15], vec![6])));
    t.eq("on its own", step(&e, 7), Some((None, vec![7], vec![1])));
    t.eq("drops it", step(&e, 12), Some((Some(3), vec![12], Vec::new())));
    let (a, b) = e.hop_steps(1);
    t.eq("first frame", (a.map(|s| parts(&s)), b.map(|s| parts(&s))), (Some((None, vec![7], vec![1])), Some((Some(1), vec![8, 9], vec![2, 3]))));
    let (a, b) = e.hop_steps(9);
    t.eq("last frame", (a.map(|s| parts(&s)), b.map(|s| parts(&s))), (Some((Some(8), vec![19], vec![9])), Some((Some(9), vec![20], Vec::new()))));
    t.check("unknown frame", e.hop_steps(99) == (None, None));
    // fragments wait for each other at the port and still belong to the step that sent them
    let first = e.trace.next_hop_id();
    ping(&mut e, pcs[0], "10.0.0.2", PingOpts { count: 1, size: 3000, ..PingOpts::default() });
    e.run_idle(BUDGET);
    let frag = e.trace.events().find(|x| x.dev == pcs[0] && matches!(x.note, Note::IpFragmented { .. })).cloned();
    let s = frag.map(|x| e.step_of(&x));
    // the switch forwards the first fragment (first + 2) before the third one leaves PC1
    let ids = vec![first, first + 1, first + 3];
    t.eq("three fragments", s.as_ref().map(|s| (s.inbound, s.outbound.clone())), Some((None, ids.clone())));
    let waited: Vec<bool> = ids.iter().map(|&id| e.trace.hop(id).is_some_and(|h| h.from.0 == pcs[0] && h.t_queued < h.t_tx)).collect();
    t.eq("two waited", waited, vec![false, true, true]);
    t.check("the step of a waiting frame", e.hop_steps(first + 3).0.is_some_and(|x| Some(&x) == s.as_ref()));
}

fn hdlc(t: &mut T) {
    let (mut e, r1, _r2, ok) = serial();
    t.check("serial set-up", ok);
    ping(&mut e, r1, "10.0.12.2", PingOpts { count: 1, size: 72, ..PingOpts::default() });
    e.run_idle(BUDGET);
    let Some(h) = e.trace.hop(1).cloned() else {
        t.check("a frame on the line", false);
        return;
    };
    t.check("an hdlc frame", h.hdlc && h.bytes[..4] == [0x0F, 0, 8, 0]);
    let ls = hop_layers(&h);
    let shape: Vec<(String, u32, u32)> = ls.iter().map(|l| (l.title.clone(), l.off, l.len)).collect();
    let want = vec![
        (String::from("Cisco HDLC"), 0, 4),
        (String::from("Internet Protocol Version 4, Src: 10.0.12.1, Dst: 10.0.12.2"), 4, 20),
        (String::from("Internet Control Message Protocol"), 24, 80),
    ];
    t.eq("hdlc layers", shape, want);
    let field = |li: usize, name: &str| ls.get(li).and_then(|l| l.fields.iter().find(|f| f.name == name)).map(|f| (f.value.clone(), f.off, f.len));
    t.eq("hdlc address", field(0, "Address"), Some((String::from("0x0f (unicast)"), 0, 1)));
    t.eq("hdlc protocol", field(0, "Protocol"), Some((String::from("IPv4 (0x0800)"), 2, 2)));
    t.eq("source behind hdlc", field(1, "Source Address"), Some((String::from("10.0.12.1"), 16, 4)));
    t.check("checksum checked", field(1, "Header Checksum").is_some_and(|f| f.0.ends_with("[correct]")) && ls.iter().all(|l| l.problems.is_empty()));
    let (top, info) = hop_info(&h);
    t.check("hdlc summary", top == Proto::Icmp && info.starts_with("Echo") && info.contains("seq=1, ttl=255"));
    let (e2, ..) = lan3_ping();
    t.check("ethernet as before", e2.trace.hop(6).is_some_and(|h| hop_layers(h) == decode::layers(&h.bytes) && hop_info(h) == decode::info(&h.bytes)));
}

/// the types of the blocks of a pcapng file, after checking that each block's length fits
fn blocks(b: &[u8]) -> Option<Vec<u32>> {
    let mut v = Vec::new();
    let mut o = 0;
    while o < b.len() {
        let rd = |at: usize| b.get(at..at + 4).map(|x| u32::from_le_bytes([x[0], x[1], x[2], x[3]]));
        let (typ, len) = (rd(o)?, rd(o + 4)? as usize);
        if len < 12 || len % 4 != 0 || rd(o + len - 4)? as usize != len {
            return None;
        }
        v.push(typ);
        o += len;
    }
    Some(v)
}

fn capture(t: &mut T) {
    let (e, pcs, _sw, _) = lan3_ping();
    let pc = e.pcapng(&PcapOpts::default());
    t.eq("every frame", pc.packets, 9);
    t.check("section header", pc.bytes.len() > 28 && pc.bytes[..4] == [0x0A, 0x0D, 0x0D, 0x0A] && pc.bytes[8..12] == [0x4D, 0x3C, 0x2B, 0x1A]);
    t.eq("blocks", blocks(&pc.bytes), Some(vec![0x0A0D_0D0A, 1, 6, 1, 6, 1, 6, 6, 6, 6, 6, 6, 6]));
    let Ok(cap) = pcapng::read(&pc.bytes) else {
        t.check("readable", false);
        return;
    };
    t.eq("written by", cap.app.as_str(), "JFNetworker");
    let ifs: Vec<(u16, &str, u64)> = cap.interfaces.iter().map(|i| (i.link, i.name.as_str(), i.units)).collect();
    t.eq("one interface per cable", ifs, vec![(1, "PC1 eth0 \u{2013} SW1 Gi1/0/1", 1_000_000_000), (1, "PC2 eth0 \u{2013} SW1 Gi1/0/2", 1_000_000_000), (1, "PC3 eth0 \u{2013} SW1 Gi1/0/3", 1_000_000_000)]);
    let pk: Vec<(u32, u64)> = cap.packets.iter().map(|p| (p.iface, p.t_ns)).collect();
    t.eq("virtual timestamps", pk, vec![(0, 0), (1, 722), (2, 722), (1, 1444), (0, 2166), (0, 2888), (1, 3914), (1, 4940), (0, 5966)]);
    t.check("the recorded bytes", cap.packets.iter().zip(e.trace.hops()).all(|(p, h)| p.data == *h.bytes && p.orig_len as usize == h.bytes.len()));
    t.eq("comments", cap.packets.get(1).map(|p| p.comment.as_str()), Some("SW1 Gi1/0/2 \u{2192} PC2 eth0"));
    // one cable, one device, a filter, a window
    let only = |o: PcapOpts| pcapng::read(&e.pcapng(&o).bytes).unwrap_or_default();
    let cable = only(PcapOpts { scope: PcapScope::Link(6), ..PcapOpts::default() });
    t.eq("one cable", (cable.interfaces.len(), cable.packets.iter().map(|p| p.t_ns).collect::<Vec<_>>()), (1, vec![722, 1444, 3914, 4940]));
    let dev = only(PcapOpts { scope: PcapScope::Device(pcs[1]), ..PcapOpts::default() });
    t.eq("one device", dev.interfaces.iter().map(|i| i.name.as_str()).collect::<Vec<_>>(), vec!["PC2 eth0"]);
    let ids: Vec<u64> = [2, 4, 7, 8].to_vec();
    let got: Vec<bool> = dev.packets.iter().zip(&ids).map(|(p, &id)| e.trace.hop(id).is_some_and(|h| p.data == *h.bytes)).collect();
    t.eq("received at t_rx, sent at t_tx", (dev.packets.iter().map(|p| p.t_ns).collect::<Vec<_>>(), got), (vec![1444, 1444, 4940, 4940], vec![true; 4]));
    let icmp = only(PcapOpts { filter: Filter::protocols(P_ICMP), ..PcapOpts::default() });
    t.eq("icmp only", icmp.packets.iter().map(|p| p.t_ns).collect::<Vec<_>>(), vec![2888, 3914, 4940, 5966]);
    let window = only(PcapOpts { from: 2000, until: 4000, ..PcapOpts::default() });
    t.eq("a window", window.packets.iter().map(|p| p.t_ns).collect::<Vec<_>>(), vec![2166, 2888, 3914]);
    let quiet = only(PcapOpts { comments: false, ..PcapOpts::default() });
    t.check("without comments", quiet.packets.len() == 9 && quiet.packets.iter().all(|p| p.comment.is_empty()));
    t.eq("empty", only(PcapOpts { scope: PcapScope::Link(99), ..PcapOpts::default() }).packets.len(), 0);
    // a serial line is Cisco HDLC
    let (mut s, r1, _r2, _) = serial();
    ping(&mut s, r1, "10.0.12.2", once());
    s.run_idle(BUDGET);
    let ser = pcapng::read(&s.pcapng(&PcapOpts::default()).bytes).unwrap_or_default();
    t.eq("hdlc link type", ser.interfaces.iter().map(|i| (i.link, i.name.as_str())).collect::<Vec<_>>(), vec![(pcapng::LINKTYPE_C_HDLC, "R1 Se0/2/0 \u{2013} R2 Se0/2/0")]);
    t.check("hdlc frames", ser.packets.len() == 2 && ser.packets.iter().all(|p| p.data[..4] == [0x0F, 0, 8, 0]));
    // a lost frame says so, and its receiver never got it
    let (mut l, pcs, sw, _) = lan3();
    let _ = l.apply(Action::Impair { link: 5, kbps: None, delay_ns: None, jitter_ns: None, loss_ppm: Some(1_000_000) });
    ping(&mut l, pcs[0], "10.0.0.2", once());
    l.run_until(SEC / 2, BUDGET);
    let lost = pcapng::read(&l.pcapng(&PcapOpts::default()).bytes).unwrap_or_default();
    t.eq("lost", lost.packets.first().map(|p| p.comment.as_str()), Some("PC1 eth0 \u{2192} SW1 Gi1/0/1, auf der Leitung verloren"));
    t.eq("not received", l.pcapng(&PcapOpts { scope: PcapScope::Device(sw), ..PcapOpts::default() }).packets, 0);
    t.eq("but sent", l.pcapng(&PcapOpts { scope: PcapScope::Device(pcs[0]), ..PcapOpts::default() }).packets, 1);
}

/// capture files of several kinds of traffic (for tshark with --pcap-out)
fn capture_files(t: &mut T) {
    // a LAN: ARP, echo, fragments
    let (mut e, pcs, _sw, _) = lan3_ping();
    ping(&mut e, pcs[0], "10.0.0.2", PingOpts { count: 1, size: 3000, ..PingOpts::default() });
    e.run_idle(BUDGET);
    let lan = e.pcapng(&PcapOpts::default());
    t.check("fragments captured", lan.packets > 9 && pcapng::read(&lan.bytes).is_ok_and(|c| c.packets.len() == lan.packets));
    save("e7-lan.pcapng", &lan.bytes);
    save("e7-device.pcapng", &e.pcapng(&PcapOpts { scope: PcapScope::Device(pcs[1]), ..PcapOpts::default() }).bytes);
    // routed: ping and traceroute (UDP, time exceeded, port unreachable)
    let (mut r, pc1, _r1, _pc2, _) = routed(true);
    ping(&mut r, pc1, "10.0.2.10", PingOpts { count: 2, ..PingOpts::default() });
    r.run_idle(BUDGET);
    let _ = r.traceroute(pc1, ip("10.0.2.10"), TraceOpts::default());
    r.run_idle(BUDGET);
    let rt = r.pcapng(&PcapOpts::default());
    t.check("routed traffic", rt.packets > 20 && r.trace.hops().any(|h| h.mask & P_UDP != 0));
    save("e7-routed.pcapng", &rt.bytes);
    // 802.1Q tags on the trunk to a router on a stick
    let (mut s, pc1, _sw, _r1, ok) = stick(true);
    t.check("stick set-up", ok);
    ping(&mut s, pc1, "10.0.20.10", PingOpts { count: 2, ..PingOpts::default() });
    s.run_idle(BUDGET);
    let tagged = s.pcapng(&PcapOpts::default());
    t.check("tagged frames", s.trace.hops().any(|h| h.mask & decode::P_VLAN != 0) && tagged.packets > 10);
    save("e7-stick.pcapng", &tagged.bytes);
    // a serial line
    let (mut w, r1, _r2, _) = serial();
    ping(&mut w, r1, "10.0.12.2", PingOpts { count: 2, ..PingOpts::default() });
    w.run_idle(BUDGET);
    save("e7-serial.pcapng", &w.pcapng(&PcapOpts::default()).bytes);
}

/// the problems of one kind: (device, port, others, text)
fn of_kind(e: &Engine, k: ProblemKind) -> Vec<(u32, Option<u16>, Vec<u32>, String)> {
    e.problems().into_iter().filter(|p| p.kind == k).map(|p| (p.dev, p.port, p.others, p.text)).collect()
}

fn problems_ip(t: &mut T) {
    let (e, _pc1, _r1, _pc2, ok) = routed(true);
    t.check("routed set-up", ok);
    t.eq("nothing wrong", e.problems(), Vec::new());
    t.eq("a LAN without a router needs no gateway", lan3().0.problems(), Vec::new());
    // duplicate addresses in one segment, but not across VLANs
    let (p, pcs, sw) = lan3_project();
    let [_pc1, pc2, pc3] = pcs;
    let mut e = engine(&p);
    let _ = addr(&mut e, pcs[0], "eth0", "10.0.0.1/24") && addr(&mut e, pc2, "eth0", "10.0.0.2/24") && addr(&mut e, pc3, "eth0", "10.0.0.2/24");
    let want = vec![
        (pc2, Some(0), vec![pc3], String::from("10.0.0.2 an eth0 ist im selben Netz doppelt vergeben: auch PC3 (eth0).")),
        (pc3, Some(0), vec![pc2], String::from("10.0.0.2 an eth0 ist im selben Netz doppelt vergeben: auch PC2 (eth0).")),
    ];
    t.eq("duplicate address", of_kind(&e, ProblemKind::DuplicateIp), want);
    t.check("an error", e.problems().iter().all(|p| p.severity == Severity::Error && p.link.is_none()));
    let _ = e.apply(Action::Access { dev: sw, port: String::from("gi1/0/3"), vlan: 10 });
    t.eq("another VLAN is another network", e.problems(), Vec::new());
    // masks: the odd one out, or both ends of a link
    let mut e = engine(&p);
    let _ = addr(&mut e, pcs[0], "eth0", "10.0.0.1/24") && addr(&mut e, pc2, "eth0", "10.0.0.2/24") && addr(&mut e, pc3, "eth0", "10.0.0.3/25");
    t.eq("mask of one host", of_kind(&e, ProblemKind::MaskMismatch), vec![(pc3, Some(0), vec![pcs[0], pc2], String::from("Netzmaske passt nicht: eth0 hat 10.0.0.3/25, PC1 (eth0) im selben Netz /24."))]);
    let mut q = Project::new("p2p", 25);
    let r1 = q.add_device("jf-r4", 0, 0).unwrap_or(0);
    let r2 = q.add_device("jf-r4", 100, 0).unwrap_or(0);
    let _ = q.connect(&pr(r1, "gi0/0/0"), &pr(r2, "gi0/0/0"), None);
    let mut e = engine(&q);
    let _ = addr(&mut e, r1, "gi0/0/0", "10.0.12.1/24") && addr(&mut e, r2, "gi0/0/0", "10.0.12.2/30");
    let want = vec![
        (r1, Some(0), vec![r2], String::from("Netzmaske passt nicht: Gi0/0/0 hat 10.0.12.1/24, R2 (Gi0/0/0) im selben Netz /30.")),
        (r2, Some(0), vec![r1], String::from("Netzmaske passt nicht: Gi0/0/0 hat 10.0.12.2/30, R1 (Gi0/0/0) im selben Netz /24.")),
    ];
    t.eq("masks on a link", of_kind(&e, ProblemKind::MaskMismatch), want);
    // shut down with an address (and IOS says how to fix it)
    let want = vec![
        (r1, Some(0), Vec::new(), String::from("Gi0/0/0 hat die Adresse 10.0.12.1/24, ist aber abgeschaltet (\u{201e}no shutdown\u{201c} fehlt).")),
        (r2, Some(0), Vec::new(), String::from("Gi0/0/0 hat die Adresse 10.0.12.2/30, ist aber abgeschaltet (\u{201e}no shutdown\u{201c} fehlt).")),
    ];
    t.eq("shut down", of_kind(&e, ProblemKind::ShutWithAddress), want);
    t.check("warnings", e.problems().iter().all(|p| p.severity == Severity::Warning));
    let (mut s, pcs, sw, _) = lan3();
    let _ = addr(&mut s, sw, "vlan1", "10.0.0.254/24");
    t.eq("an svi shut down", of_kind(&s, ProblemKind::ShutWithAddress), vec![(sw, None, Vec::new(), String::from("Vlan1 hat die Adresse 10.0.0.254/24, ist aber abgeschaltet (\u{201e}no shutdown\u{201c} fehlt)."))]);
    t.check("pcs untouched", !s.problems().iter().any(|p| pcs.contains(&p.dev)));
    // gateways: missing although a router is there, outside the network, nobody has it
    let (mut e, pc1, r1, pc2, _) = routed(true);
    let _ = gw(&mut e, pc1, "10.0.1.1", false);
    let _ = gw(&mut e, pc2, "10.0.2.1", false) && gw(&mut e, pc2, "10.0.3.1", true);
    t.eq("no gateway", of_kind(&e, ProblemKind::NoGateway), vec![(pc1, Some(0), vec![r1], String::from("Kein Standard-Gateway: PC1 erreicht nur das eigene Netz (R1 mit 10.0.1.1 könnte es sein)."))]);
    t.eq("gateway outside", of_kind(&e, ProblemKind::GatewayOffNet), vec![(pc2, Some(0), Vec::new(), String::from("Das Standard-Gateway 10.0.3.1 liegt außerhalb der eigenen Netze (10.0.2.0/24)."))]);
    let _ = gw(&mut e, pc1, "10.0.1.254", true);
    t.eq("unknown gateway", of_kind(&e, ProblemKind::GatewayUnknown), vec![(pc1, Some(0), Vec::new(), String::from("Das Standard-Gateway 10.0.1.254 ist im Netz von eth0 nicht zu finden: kein Gerät dort hat diese Adresse."))]);
    let all = e.problems();
    t.check("errors first", all.first().is_some_and(|p| p.severity == Severity::Error) && all.last().is_some_and(|p| p.severity == Severity::Warning));
    t.eq("severity names", (Severity::Error.title(), Severity::Warning.title()), ("Fehler", "Warnung"));
    // a shut router port still holds the gateway's address
    let (e, ..) = routed(false);
    t.eq("shut router ports", of_kind(&e, ProblemKind::ShutWithAddress).len(), 2);
    t.check("their gateway exists", of_kind(&e, ProblemKind::GatewayUnknown).is_empty());
}

/// two switches joined Te1/1/1 - Te1/1/1 (cable 3), set up by `f`
fn pair(f: &dyn Fn(&mut Engine, u32, u32)) -> (Engine, u32, u32) {
    let mut p = Project::new("pair", 26);
    let s1 = p.add_device("jf-s24p", 0, 0).unwrap_or(0);
    let s2 = p.add_device("jf-s24p", 100, 0).unwrap_or(0);
    let _ = p.connect(&pr(s1, "te1/1/1"), &pr(s2, "te1/1/1"), None);
    let mut e = engine(&p);
    f(&mut e, s1, s2);
    (e, s1, s2)
}

fn trunk(e: &mut Engine, dev: u32, native: u16, allowed: &str) {
    let _ = e.apply(Action::Trunk { dev, port: String::from("te1/1/1"), native, allowed: VlanSet::parse(allowed).unwrap_or_default() });
}

fn access(e: &mut Engine, dev: u32, port: &str, vlan: u16) {
    let _ = e.apply(Action::Access { dev, port: String::from(port), vlan });
}

fn problems_l2(t: &mut T) {
    let link = |e: &Engine| e.problems().into_iter().map(|p| (p.kind, p.severity, p.dev, p.port, p.link, p.others, p.text)).collect::<Vec<_>>();
    let (e, s1, s2) = pair(&|e, a, b| {
        trunk(e, a, 1, "all");
        trunk(e, b, 99, "all");
    });
    let text = String::from("Native VLAN passt nicht: SW1 Te1/1/1 hat VLAN 1, SW2 Te1/1/1 VLAN 99; ungetaggte Frames wechseln auf dem Kabel das VLAN.");
    t.eq("native vlan", link(&e), vec![(ProblemKind::NativeVlanMismatch, Severity::Warning, s1, Some(24), Some(3), vec![s2], text)]);
    let (e, ..) = pair(&|e, a, b| {
        trunk(e, a, 1, "10");
        trunk(e, b, 1, "20");
    });
    t.eq("no common vlan", link(&e).iter().map(|p| (p.0, p.1, p.6.as_str())).collect::<Vec<_>>(), vec![(ProblemKind::NoCommonVlan, Severity::Error, "Der Trunk zwischen SW1 Te1/1/1 und SW2 Te1/1/1 lässt kein VLAN auf beiden Seiten zu.")]);
    let (e, ..) = pair(&|e, a, b| {
        access(e, a, "te1/1/1", 10);
        trunk(e, b, 1, "all");
    });
    t.eq("access against trunk", of_kind(&e, ProblemKind::AccessTrunk), vec![(s1, Some(24), vec![s2], String::from("SW1 Te1/1/1 ist Access-Port (VLAN 10), SW2 Te1/1/1 ist Trunk."))]);
    let (e, ..) = pair(&|e, a, b| {
        trunk(e, a, 1, "all");
        access(e, b, "te1/1/1", 20);
    });
    t.eq("trunk against access", of_kind(&e, ProblemKind::AccessTrunk), vec![(s2, Some(24), vec![s1], String::from("SW2 Te1/1/1 ist Access-Port (VLAN 20), SW1 Te1/1/1 ist Trunk."))]);
    let (e, ..) = pair(&|e, a, b| {
        access(e, a, "te1/1/1", 10);
        access(e, b, "te1/1/1", 20);
    });
    t.eq("access vlans", of_kind(&e, ProblemKind::NativeVlanMismatch).iter().map(|p| p.3.as_str()).collect::<Vec<_>>(), vec!["VLAN passt nicht: SW1 Te1/1/1 ist Access-Port in VLAN 10, SW2 Te1/1/1 in VLAN 20; die Frames wechseln auf dem Kabel das VLAN."]);
    let (e, ..) = pair(&|e, a, b| {
        trunk(e, a, 1, "all");
        trunk(e, b, 1, "all");
        access(e, a, "gi1/0/1", 10);
    });
    t.eq("a matching trunk is fine, and no loop", e.problems(), Vec::new());
    // a router on a stick against an access port
    let (e, _pc1, sw, r1, _) = stick(false);
    t.eq("subinterfaces against access", of_kind(&e, ProblemKind::AccessTrunk), vec![(sw, Some(23), vec![r1], String::from("R1 Gi0/0/0 hat 802.1Q-Subinterfaces, SW1 Gi1/0/24 ist aber Access-Port (VLAN 1): getaggte Frames verwirft der Switch."))]);
    t.eq("on a trunk it is fine", stick(true).0.problems(), Vec::new());
    // the wrong copper cable matters only with strict MDI-X
    let mut ps = Project::new("mdix", 27);
    let pc = ps.add_device("pc", 0, 0).unwrap_or(0);
    let sw = ps.add_device("jf-s24p", 0, 100).unwrap_or(0);
    let lx = ps.connect(&pr(pc, "eth0"), &pr(sw, "gi1/0/1"), Some(CableKind::CopperCross)).unwrap_or(0);
    t.eq("auto-mdix", engine(&ps).problems(), Vec::new());
    ps.strict_mdix = true;
    let text = String::from("Falsches Kabel zwischen PC1 eth0 und SW1 Gi1/0/1: hierher gehört Kupfer, gerade; ohne Auto-MDI-X bleibt die Leitung aus.");
    t.eq("wrong cable", link(&engine(&ps)), vec![(ProblemKind::WrongCable, Severity::Error, pc, Some(0), Some(lx), vec![sw], text)]);
    // loops: a triangle, two parallel cables, a hub; a shut port breaks a loop
    let tri = |f: &dyn Fn(&mut Engine, [u32; 3])| {
        let mut p = Project::new("tri", 28);
        let s = [0, 1, 2].map(|i| p.add_device("jf-s24p", i * 100, 0).unwrap_or(0));
        let _ = p.connect(&pr(s[0], "gi1/0/1"), &pr(s[1], "gi1/0/1"), None);
        let _ = p.connect(&pr(s[1], "gi1/0/2"), &pr(s[2], "gi1/0/1"), None);
        let _ = p.connect(&pr(s[2], "gi1/0/2"), &pr(s[0], "gi1/0/2"), None);
        let mut e = engine(&p);
        f(&mut e, s);
        (e, s)
    };
    let (e, s) = tri(&|_, _| {});
    let text = String::from("Schleife ohne Spanning Tree über SW1, SW2, SW3 (VLAN 1): Broadcasts kreisen endlos.");
    t.eq("triangle", link(&e), vec![(ProblemKind::Loop, Severity::Error, s[0], Some(1), Some(6), vec![s[1], s[2]], text)]);
    let (e, _) = tri(&|e, s| {
        let _ = e.apply(Action::PortAdmin { dev: s[2], port: String::from("gi1/0/2"), up: false });
    });
    t.eq("a shut port breaks it", e.problems(), Vec::new());
    let (e, _) = tri(&|e, s| {
        for (d, ports) in [(s[0], ["gi1/0/1", "gi1/0/2"]), (s[1], ["gi1/0/1", "gi1/0/2"]), (s[2], ["gi1/0/1", "gi1/0/2"])] {
            for port in ports {
                let _ = e.apply(Action::Trunk { dev: d, port: String::from(port), native: 1, allowed: VlanSet::all() });
            }
        }
        access(e, s[0], "gi1/0/10", 10);
    });
    t.eq("a triangle of trunks, once", of_kind(&e, ProblemKind::Loop).iter().map(|p| p.3.as_str()).collect::<Vec<_>>(), vec!["Schleife ohne Spanning Tree über SW1, SW2, SW3 (VLAN 1, 10): Broadcasts kreisen endlos."]);
    let mut p = Project::new("twice", 29);
    let a = p.add_device("jf-s24p", 0, 0).unwrap_or(0);
    let b = p.add_device("jf-s24p", 100, 0).unwrap_or(0);
    let _ = p.connect(&pr(a, "gi1/0/1"), &pr(b, "gi1/0/1"), None);
    let _ = p.connect(&pr(a, "gi1/0/2"), &pr(b, "gi1/0/2"), None);
    t.eq("two cables", of_kind(&engine(&p), ProblemKind::Loop).iter().map(|p| p.3.as_str()).collect::<Vec<_>>(), vec!["Schleife ohne Spanning Tree über SW1, SW2 (VLAN 1): Broadcasts kreisen endlos."]);
    let mut p = Project::new("hub", 30);
    let hub = p.add_device("hub", 0, 0).unwrap_or(0);
    let sw = p.add_device("jf-s24p", 100, 0).unwrap_or(0);
    let _ = p.connect(&pr(hub, "port0"), &pr(sw, "gi1/0/1"), None);
    let _ = p.connect(&pr(hub, "port1"), &pr(sw, "gi1/0/2"), None);
    let loops = of_kind(&engine(&p), ProblemKind::Loop);
    t.eq("through a hub", loops.iter().map(|p| (p.0, p.2.clone())).collect::<Vec<_>>(), vec![(hub, vec![sw])]);
}

#[cfg(any(windows, target_os = "linux"))]
fn threaded(t: &mut T) {
    use crate::sim_thread::{self, Threaded};
    use jfn_sim::{ActErr, Mode};

    let frames = |e: &Engine| e.trace.hops().map(|h| (h.t_tx, h.t_rx, h.from, h.to, h.bytes.to_vec())).collect::<Vec<_>>();
    // the same run inline, for comparison
    let (e, pcs, _sw, _) = lan3();
    let mut inl = Inline::new(e.clone());
    let _ = inl.eng.ping(pcs[0], ip("10.0.0.2"), once());
    let _ = inl.send(Cmd::RunIdle, 0);
    let Some(mut d) = Threaded::new(e) else {
        t.check("worker started", false);
        return;
    };
    t.check("starts paused", d.mode() == Mode::Paused && !d.busy());
    t.eq("errors at once", d.send(Cmd::Apply(Action::Power { dev: 99, on: true }), 0), Err(ActErr::UnknownDevice(99)));
    let _ = d.lock().eng.ping(pcs[0], ip("10.0.0.2"), once());
    let r = d.send(Cmd::RunIdle, 0);
    t.check("a long run goes to the worker", r.is_ok_and(|x| x.events == 0));
    t.check("done in time", d.wait_idle(20_000));
    t.check("nothing pending", !d.busy() && !sim_thread::pending());
    let tk = d.tick(0);
    t.check("the tick reports it", tk.events > 0 && tk.new_hops && tk.now == 2 * SEC && tk.wake_ms.is_none());
    t.eq("taken once", d.tick(0).events, 0);
    t.check("the same frames as inline", frames(&d.lock().eng) == frames(&inl.eng));
    // simulation steps answer at once, as inline
    let (e, pcs, _sw, _) = lan3();
    let Some(mut s) = Threaded::new(e) else {
        return;
    };
    let _ = s.send(Cmd::Simulation, 0);
    let _ = s.lock().eng.ping(pcs[0], ip("10.0.0.2"), once());
    t.eq("a step", s.send(Cmd::Step(Filter::all()), 0).map(|x| (x.now, x.new_hops, x.view)), Ok((722, true, None)));
    let _ = s.send(Cmd::Step(Filter::all()), 0);
    t.eq("a step back", s.send(Cmd::StepBack(Filter::all()), 0).map(|x| x.view), Ok(Some(722)));
    t.check("problems from the worker's engine", s.lock().eng.problems().is_empty());
    // realtime: the worker moves the clock on its own, a pause stops it
    let _ = s.send(Cmd::Realtime { speed_pm: 1000 }, plat::task::now_us());
    let end = plat::time_us() + 5_000_000;
    while s.lock().eng.now() < 10 * MS && plat::time_us() < end {
        plat::sleep_ms(2);
    }
    let moved = s.lock().eng.now();
    t.check("realtime moves the clock", moved > 1444);
    let _ = s.send(Cmd::Pause, 0);
    let paused = s.lock().eng.now();
    plat::sleep_ms(30);
    t.check("a pause stops it", s.lock().eng.now() == paused && s.mode() == Mode::Paused);
    let tk = s.tick(0);
    t.check("realtime reported its frames", tk.events > 0 && tk.new_hops);
    drop(s);
    drop(d);
    t.check("stopped", !sim_thread::pending());
}
