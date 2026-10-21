// simtest -- checks of jfn_sim (part of --selftest): timing on the wire, learning and
// flooding, VLANs on access and trunk ports, aging, link state, loss, queues, hubs, power,
// removal while frames are under way, determinism and the inline driver

use alloc::string::String;
use alloc::vec::Vec;

use jfn_model::project::{PortRef, Project};
use jfn_model::CableKind;
use jfn_sim::node::port_mac;
use jfn_sim::{Action, Cmd, Engine, HopDrop, Inline, Led, Note, VlanSet, SEC};
use netpkt::decode;
use netpkt::eth::{self, EthHdr};
use netpkt::Mac;

use crate::selftest::T;

#[path = "l3test.rs"]
pub mod l3test;

#[path = "simmodetest.rs"]
pub mod simmodetest;
#[path = "svctest.rs"]
pub mod svctest;
#[path = "clitest.rs"]
pub mod clitest;
#[path = "tcptest.rs"]
pub mod tcptest;
#[path = "httptest.rs"]
pub mod httptest;
#[path = "pdutest.rs"]
pub mod pdutest;
#[path = "labtest.rs"]
pub mod labtest;
#[path = "campustest.rs"]
pub mod campustest;
#[path = "synthtest.rs"]
pub mod synthtest;

const BUDGET: usize = 1_000_000;

fn pr(dev: u32, port: &str) -> PortRef {
    PortRef { dev, port: String::from(port) }
}

/// an Ethernet frame of `len` bytes (local experimental ethertype, zero payload)
fn frame(dst: Mac, src: Mac, len: usize) -> Vec<u8> {
    EthHdr { dst, src, ethertype: 0x88B5 }.frame(&alloc::vec![0u8; len.saturating_sub(14)])
}

fn mac(dev: u32) -> Mac {
    port_mac(dev, 0)
}

/// PC1..PCn on SW (Gi1/0/1..n); returns (project, pcs, switch)
fn lan(n: usize) -> (Project, Vec<u32>, u32) {
    let mut p = Project::new("lan", 7);
    let pcs: Vec<u32> = (0..n).map(|i| p.add_device("pc", i as i32 * 100, 0).unwrap_or(0)).collect();
    let sw = p.add_device("jf-s24p", 0, 100).unwrap_or(0);
    for (i, &pc) in pcs.iter().enumerate() {
        let _ = p.connect(&pr(pc, "eth0"), &pr(sw, &alloc::format!("gi1/0/{}", i + 1)), None);
    }
    (p, pcs, sw)
}

fn engine(p: &Project) -> Engine {
    Engine::from_project(p).unwrap_or_else(|_| Engine::new(0))
}

fn inject(e: &mut Engine, dev: u32, port: &str, f: Vec<u8>) -> bool {
    e.apply(Action::Inject { dev, port: String::from(port), frame: f }).is_ok()
}

/// the notes of a device, in order
fn notes(e: &Engine, dev: u32) -> Vec<Note> {
    e.trace.events().filter(|x| x.dev == dev).map(|x| x.note.clone()).collect()
}

fn link_of(p: &Project, dev: u32) -> u32 {
    p.neighbors(dev).first().map(|n| n.3).unwrap_or(0)
}

pub fn run(t: &mut T) {
    flood_learn(t);
    timing(t);
    vlans(t);
    link_state(t);
    loss_queue_hub(t);
    power_remove(t);
    determinism(t);
    driver(t);
}

fn flood_learn(t: &mut T) {
    let (p, pcs, sw) = lan(3);
    let mut e = engine(&p);
    t.eq("devices and links", (e.devices().count(), e.view().links.len()), (4, 3));
    t.check("links up", e.view().links.iter().all(|l| l.up && l.led == [Led::Up, Led::Up]));
    // a broadcast from PC1 floods to PC2 and PC3 and teaches the switch PC1's port
    t.check("inject", inject(&mut e, pcs[0], "eth0", frame(Mac::BROADCAST, mac(pcs[0]), 64)));
    e.run_idle(BUDGET);
    t.eq("broadcast hops", e.trace.hops().count(), 3);
    t.eq("switch notes", notes(&e, sw), alloc::vec![Note::LinkUp { port: 0 }, Note::LinkUp { port: 1 }, Note::LinkUp { port: 2 }, Note::Learned { mac: mac(pcs[0]), port: 0, vlan: 1 }, Note::Flooded { vlan: 1, ports: 2 }]);
    t.check("both got it", notes(&e, pcs[1]).contains(&Note::Delivered { port: 0 }) && notes(&e, pcs[2]).contains(&Note::Delivered { port: 0 }));
    t.eq("mac table", e.bridge(sw).map(|b| b.lookup(1, mac(pcs[0]), e.now())), Some(Some(0)));
    // a known unicast goes to one port
    let before = e.trace.next_hop_id();
    inject(&mut e, pcs[1], "eth0", frame(mac(pcs[0]), mac(pcs[1]), 64));
    e.run_idle(BUDGET);
    let hops: Vec<(u32, u32)> = e.trace.hops().filter(|h| h.id >= before).map(|h| (h.from.0, h.to.0)).collect();
    t.eq("unicast path", hops, alloc::vec![(pcs[1], sw), (sw, pcs[0])]);
    t.check("forwarded", notes(&e, sw).contains(&Note::Forwarded { vlan: 1, port: 0 }) && notes(&e, pcs[0]).contains(&Note::Delivered { port: 0 }));
    // an unknown unicast floods and the hosts throw it away
    inject(&mut e, pcs[2], "eth0", frame(Mac([2, 0, 0, 0, 0, 0x99]), mac(pcs[2]), 64));
    e.run_idle(BUDGET);
    t.check("unknown floods", notes(&e, pcs[0]).contains(&Note::NotForMe { port: 0 }) && notes(&e, pcs[1]).contains(&Note::NotForMe { port: 0 }));
    // a frame back to its own port is filtered
    let mut e2 = engine(&p);
    inject(&mut e2, pcs[0], "eth0", frame(Mac::BROADCAST, mac(pcs[0]), 64));
    e2.run_idle(BUDGET);
    inject(&mut e2, pcs[0], "eth0", frame(mac(pcs[0]), Mac([2, 0, 0, 0, 0, 0x55]), 64));
    e2.run_idle(BUDGET);
    t.check("filtered", notes(&e2, sw).contains(&Note::Filtered { vlan: 1 }));
    // link-local control frames stay in the bridge
    inject(&mut e2, pcs[0], "eth0", frame(Mac::STP, mac(pcs[0]), 64));
    e2.run_idle(BUDGET);
    t.check("stp consumed", notes(&e2, sw).contains(&Note::Consumed { port: 0 }));
    // aging: after 300 s the switch floods again
    let mut e3 = engine(&p);
    inject(&mut e3, pcs[0], "eth0", frame(Mac::BROADCAST, mac(pcs[0]), 64));
    e3.run_idle(BUDGET);
    let t0 = e3.now();
    e3.run_until(t0 + 301 * SEC, BUDGET);
    inject(&mut e3, pcs[1], "eth0", frame(mac(pcs[0]), mac(pcs[1]), 64));
    e3.run_idle(BUDGET);
    t.check("aged out", notes(&e3, sw).last() == Some(&Note::Flooded { vlan: 1, ports: 2 }));
    t.eq("aged entries hidden", e3.bridge(sw).map(|b| b.entries(e3.now()).len()), Some(1));
}

fn timing(t: &mut T) {
    let (p, pcs, sw) = lan(2);
    let mut e = engine(&p);
    inject(&mut e, pcs[0], "eth0", frame(Mac::BROADCAST, mac(pcs[0]), 100));
    inject(&mut e, pcs[0], "eth0", frame(Mac::BROADCAST, mac(pcs[0]), 20));
    e.run_idle(BUDGET);
    let h: Vec<(u32, u64, u64)> = e.trace.hops().map(|h| (h.from.0, h.t_tx, h.t_rx)).collect();
    // 1 Gbit/s: (100 + 24) * 8 = 992 ns on the wire, 50 ns cable; the second frame waits for
    // the first and is padded to 60 bytes: (60 + 24) * 8 = 672 ns; the switch stores and
    // forwards each when it has arrived, the second when its port is free again (2034 ns)
    t.eq("wire times", h, alloc::vec![(pcs[0], 0, 1042), (pcs[0], 992, 1714), (sw, 1042, 2084), (sw, 2034, 2756)]);
    t.eq("link speed", e.view().links[0].kbps, 1_000_000);
    t.eq("serialisation 10G", jfn_sim::link::serialisation_ns(1500, 10_000_000, true), 1219);
    t.eq("serialisation serial", jfn_sim::link::serialisation_ns(1000, 2048, false), 3_906_250);
}

fn vlans(t: &mut T) {
    let (mut p, pcs, sw1) = lan(4);
    let sw2 = p.add_device("jf-s24p", 500, 100).unwrap_or(0);
    let pc5 = p.add_device("pc", 500, 0).unwrap_or(0);
    let pc6 = p.add_device("pc", 600, 0).unwrap_or(0);
    let _ = p.connect(&pr(pc5, "eth0"), &pr(sw2, "gi1/0/1"), None);
    let _ = p.connect(&pr(pc6, "eth0"), &pr(sw2, "gi1/0/2"), None);
    let trunk = p.connect(&pr(sw1, "te1/1/1"), &pr(sw2, "te1/1/1"), None).unwrap_or(0);
    let mut e = engine(&p);
    let ok = [
        e.apply(Action::Access { dev: sw1, port: String::from("gi1/0/1"), vlan: 10 }),
        e.apply(Action::Access { dev: sw1, port: String::from("gi1/0/2"), vlan: 10 }),
        e.apply(Action::Access { dev: sw1, port: String::from("gi1/0/3"), vlan: 20 }),
        e.apply(Action::Access { dev: sw2, port: String::from("gi1/0/1"), vlan: 10 }),
        e.apply(Action::Trunk { dev: sw1, port: String::from("te1/1/1"), native: 1, allowed: VlanSet::all() }),
        e.apply(Action::Trunk { dev: sw2, port: String::from("te1/1/1"), native: 1, allowed: VlanSet::all() }),
    ];
    t.check("vlan set-up", ok.iter().all(|r| r.is_ok()));
    t.check("bad vlan refused", e.apply(Action::Access { dev: sw1, port: String::from("gi1/0/4"), vlan: 4095 }).is_err());
    // PC1 (VLAN 10): PC2 and, over the trunk with tag 10, PC5 get it; PC3 (20), PC4 (1), PC6 (1) do not
    inject(&mut e, pcs[0], "eth0", frame(Mac::BROADCAST, mac(pcs[0]), 64));
    e.run_idle(BUDGET);
    let got = |e: &Engine, d: u32| notes(e, d).iter().any(|n| matches!(n, Note::Delivered { .. }));
    t.check("vlan 10 reaches PC2 and PC5", got(&e, pcs[1]) && got(&e, pc5));
    t.check("other vlans stay quiet", !got(&e, pcs[2]) && !got(&e, pcs[3]) && !got(&e, pc6));
    let on_trunk: Vec<Option<u16>> = e.trace.hops().filter(|h| h.link == trunk).map(|h| eth::tag_of(&h.bytes).map(|g| g.vid)).collect();
    t.eq("tagged on the trunk", on_trunk, alloc::vec![Some(10)]);
    t.check("untagged to the host", e.trace.hops().filter(|h| h.to.0 == pc5).all(|h| eth::tag_of(&h.bytes).is_none()));
    t.check("vlan in the dissector", e.trace.hops().filter(|h| h.link == trunk).all(|h| h.mask & decode::P_VLAN != 0));
    // the native VLAN crosses untagged
    let before = e.trace.next_hop_id();
    inject(&mut e, pcs[3], "eth0", frame(Mac::BROADCAST, mac(pcs[3]), 64));
    e.run_idle(BUDGET);
    let native: Vec<Option<u16>> = e.trace.hops().filter(|h| h.id >= before && h.link == trunk).map(|h| eth::tag_of(&h.bytes).map(|g| g.vid)).collect();
    t.eq("native untagged", native, alloc::vec![None]);
    t.check("native reaches PC6", got(&e, pc6));
    // VLAN 10 no longer allowed on SW2's trunk
    let _ = e.apply(Action::Trunk { dev: sw2, port: String::from("te1/1/1"), native: 1, allowed: VlanSet::parse("1-9,11-4094").unwrap_or_default() });
    inject(&mut e, pcs[0], "eth0", frame(Mac::BROADCAST, mac(pcs[0]), 64));
    e.run_idle(BUDGET);
    t.check("not allowed dropped", notes(&e, sw2).iter().any(|n| matches!(n, Note::DropVlan { vid: 10, .. })));
    // a tagged frame on an access port
    inject(&mut e, pcs[2], "eth0", eth::tagged(&frame(Mac::BROADCAST, mac(pcs[2]), 64), netpkt::eth::VlanTag::new(20)));
    e.run_idle(BUDGET);
    t.check("tagged on access dropped", notes(&e, sw1).iter().any(|n| matches!(n, Note::DropTagged { vid: 20, .. })));
    t.eq("vlan ranges", VlanSet::parse("1-10,20,30-31").map(|s| s.to_text()), Some(String::from("1-10,20,30-31")));
    t.eq("vlan all", VlanSet::all().to_text(), String::from("1-4094"));
    t.check("vlan bad range", VlanSet::parse("0-5").is_none() && VlanSet::parse("10-4095").is_none());
}

fn link_state(t: &mut T) {
    let (p, pcs, sw) = lan(3);
    let l2 = link_of(&p, pcs[1]);
    let mut e = engine(&p);
    // learn PC2, then unplug its cable: LEDs, notes, flushed entry, no flooding to it
    inject(&mut e, pcs[1], "eth0", frame(Mac::BROADCAST, mac(pcs[1]), 64));
    e.run_idle(BUDGET);
    t.check("learned", e.bridge(sw).is_some_and(|b| b.lookup(1, mac(pcs[1]), e.now()) == Some(1)));
    t.check("unplug", e.apply(Action::Unplug { link: l2, down: true }).is_ok());
    let lv = e.view().links.into_iter().find(|l| l.id == l2);
    t.check("down in the view", lv.is_some_and(|l| !l.up && l.led == [Led::Down, Led::Down]));
    t.check("down noted", notes(&e, sw).contains(&Note::LinkDown { port: 1 }) && notes(&e, pcs[1]).contains(&Note::LinkDown { port: 0 }));
    t.check("entry flushed", e.bridge(sw).is_some_and(|b| b.lookup(1, mac(pcs[1]), e.now()).is_none()));
    let before = e.trace.next_hop_id();
    inject(&mut e, pcs[0], "eth0", frame(Mac::BROADCAST, mac(pcs[0]), 64));
    e.run_idle(BUDGET);
    t.eq("flood skips the dead port", e.trace.hops().filter(|h| h.id >= before).count(), 2);
    t.check("sender without link", inject(&mut e, pcs[1], "eth0", frame(Mac::BROADCAST, mac(pcs[1]), 64)) && notes(&e, pcs[1]).last() == Some(&Note::DropNoLink { port: 0 }));
    let _ = e.apply(Action::Unplug { link: l2, down: false });
    t.check("up again", e.view().links.iter().any(|l| l.id == l2 && l.up));
    // a shut-down port
    let _ = e.apply(Action::PortAdmin { dev: sw, port: String::from("Gi1/0/3"), up: false });
    t.eq("shut down led", (e.port_led(sw, 2), e.port_led(pcs[2], 0)), (Led::Down, Led::Down));
    let _ = e.apply(Action::PortAdmin { dev: sw, port: String::from("Gi1/0/3"), up: true });
    t.eq("no shutdown led", e.port_led(sw, 2), Led::Up);
    // routers boot with shut-down interfaces
    let mut pr_ = Project::new("r", 1);
    let r = pr_.add_device("jf-r4", 0, 0).unwrap_or(0);
    let pc = pr_.add_device("pc", 0, 0).unwrap_or(0);
    let _ = pr_.connect(&pr(r, "gi0/0/0"), &pr(pc, "eth0"), None);
    let mut er = engine(&pr_);
    t.eq("router port shut", er.port_led(r, 0), Led::Down);
    let _ = er.apply(Action::PortAdmin { dev: r, port: String::from("g0/0/0"), up: true });
    t.eq("router port up", er.port_led(r, 0), Led::Up);
    // strict MDI-X: the wrong copper cable keeps the link down
    let mut ps = Project::new("mdix", 1);
    let a = ps.add_device("pc", 0, 0).unwrap_or(0);
    let s = ps.add_device("jf-s24p", 0, 0).unwrap_or(0);
    let lx = ps.connect(&pr(a, "eth0"), &pr(s, "gi1/0/1"), Some(CableKind::CopperCross)).unwrap_or(0);
    t.check("auto-mdix works", engine(&ps).view().links.iter().all(|l| l.up && l.wrong_cable));
    ps.strict_mdix = true;
    let mut es = engine(&ps);
    t.check("strict mdix down", es.view().links.iter().all(|l| !l.up));
    t.check("right cable up", es.apply(Action::SetCable { link: lx, cable: CableKind::CopperStraight }).is_ok() && es.view().links.iter().all(|l| l.up && !l.wrong_cable));
}

fn loss_queue_hub(t: &mut T) {
    let (p, pcs, sw) = lan(2);
    let l1 = link_of(&p, pcs[0]);
    let mut e = engine(&p);
    let _ = e.apply(Action::Impair { link: l1, kbps: None, delay_ns: None, jitter_ns: None, loss_ppm: Some(1_000_000) });
    inject(&mut e, pcs[0], "eth0", frame(Mac::BROADCAST, mac(pcs[0]), 64));
    e.run_idle(BUDGET);
    t.check("lost", e.trace.hops().count() == 1 && e.trace.hops().all(|h| h.drop == Some(HopDrop::Loss)));
    t.check("switch saw nothing", !notes(&e, sw).iter().any(|n| matches!(n, Note::Flooded { .. })));
    // a slow link with a full queue drops the tail
    let _ = e.apply(Action::Impair { link: l1, kbps: Some(64), delay_ns: None, jitter_ns: None, loss_ppm: Some(0) });
    for _ in 0..1500 {
        inject(&mut e, pcs[0], "eth0", frame(Mac::BROADCAST, mac(pcs[0]), 1514));
    }
    let drops = notes(&e, pcs[0]).iter().filter(|n| matches!(n, Note::DropQueueFull { .. })).count();
    t.check("queue tail drop", drops > 0 && drops < 1500);
    t.check("queue in the view", e.view().links.iter().any(|l| l.id == l1 && l.queued > 1000));
    // a hub repeats to every other port, the hosts sort out what is theirs
    let mut ph = Project::new("hub", 3);
    let hub = ph.add_device("hub", 0, 0).unwrap_or(0);
    let hs: Vec<u32> = (0..3).map(|i| ph.add_device("pc", i, 0).unwrap_or(0)).collect();
    for (i, &h) in hs.iter().enumerate() {
        let _ = ph.connect(&pr(h, "eth0"), &pr(hub, &alloc::format!("port{}", i)), None);
    }
    let mut eh = engine(&ph);
    inject(&mut eh, hs[0], "eth0", frame(mac(hs[1]), mac(hs[0]), 64));
    eh.run_idle(BUDGET);
    t.eq("hub repeats", notes(&eh, hub).last(), Some(&Note::Repeated { ports: 2 }));
    t.check("hub hosts", notes(&eh, hs[1]).contains(&Note::Delivered { port: 0 }) && notes(&eh, hs[2]).contains(&Note::NotForMe { port: 0 }));
    t.eq("hub speed", eh.view().links[0].kbps, 100_000);
}

fn power_remove(t: &mut T) {
    let (p, pcs, sw) = lan(2);
    let mut e = engine(&p);
    inject(&mut e, pcs[0], "eth0", frame(Mac::BROADCAST, mac(pcs[0]), 64));
    e.run_idle(BUDGET);
    let _ = e.apply(Action::Power { dev: sw, on: false });
    t.eq("leds when off", (e.port_led(sw, 0), e.port_led(pcs[0], 0)), (Led::NoPower, Led::Down));
    t.check("table gone", e.bridge(sw).is_some_and(|b| b.fdb.is_empty()));
    let _ = e.apply(Action::Power { dev: sw, on: true });
    t.check("back up", e.view().links.iter().all(|l| l.up));
    // the switch disappears while a frame is on the cable
    let mut e2 = engine(&p);
    inject(&mut e2, pcs[0], "eth0", frame(Mac::BROADCAST, mac(pcs[0]), 64));
    t.eq("removed", e2.apply(Action::RemoveDevice(sw)), Ok(()));
    e2.run_idle(BUDGET);
    t.check("lost in flight", e2.trace.hops().all(|h| h.drop == Some(HopDrop::LinkDown)));
    t.check("view without it", e2.view().devices.len() == 2 && e2.view().links.is_empty());
    t.check("unknown device", e2.apply(Action::Power { dev: sw, on: true }).is_err());
    // devices and cables added while it runs
    let mut d = p.device(pcs[1]).cloned().unwrap_or_default();
    d.id = 99;
    d.name = String::from("PC99");
    t.check("add device", e2.apply(Action::AddDevice(d)).is_ok() && e2.apply(Action::RemoveDevice(99)).is_ok());
}

/// (t_tx, t_rx, link, lost) of every frame of a lossy, jittery run
fn lossy_run(seed: u64) -> Vec<(u64, u64, u32, bool)> {
    let (mut p, pcs, _) = lan(3);
    p.seed = seed;
    let mut e = engine(&p);
    for l in p.links.iter().map(|l| l.id) {
        let _ = e.apply(Action::Impair { link: l, kbps: None, delay_ns: Some(1000), jitter_ns: Some(5000), loss_ppm: Some(300_000) });
    }
    for i in 0..50 {
        inject(&mut e, pcs[i % 3], "eth0", frame(Mac::BROADCAST, mac(pcs[i % 3]), 64 + i));
        e.run_until(e.now() + 2000, BUDGET);
    }
    e.run_idle(BUDGET);
    e.trace.hops().map(|h| (h.t_tx, h.t_rx, h.link, h.drop.is_some())).collect()
}

fn determinism(t: &mut T) {
    let a = lossy_run(11);
    t.check("same seed, same run", a == lossy_run(11));
    t.check("other seed, other run", a != lossy_run(12));
    t.check("some lost, some not", a.iter().any(|h| h.3) && a.iter().any(|h| !h.3));
}

fn driver(t: &mut T) {
    let (p, pcs, _) = lan(2);
    let mut d = Inline::new(engine(&p));
    let f = frame(Mac::BROADCAST, mac(pcs[0]), 100);
    t.check("realtime", d.send(Cmd::Realtime { speed_pm: 1000 }, 0).is_ok());
    let r = d.send(Cmd::Apply(Action::Inject { dev: pcs[0], port: String::from("eth0"), frame: f.clone() }), 0);
    t.check("inject sends", r.is_ok_and(|x| x.new_hops));
    let a = d.tick(1);
    t.eq("1 us later", (a.now, a.wake_ms), (1000, Some(16)));
    t.check("not yet forwarded", d.eng.trace.hops().count() == 1);
    let b = d.tick(2);
    t.check("forwarded after arrival", b.now == 2000 && b.new_hops && d.eng.trace.hops().count() == 2);
    let c = d.send(Cmd::Pause, 3);
    t.eq("paused", c.map(|x| x.wake_ms), Ok(None));
    t.eq("paused tick", d.tick(1_000_000).now, 2000);
    // simulation mode: one step per frame
    let mut s = Inline::new(engine(&p));
    let _ = s.send(Cmd::Simulation, 0);
    let _ = s.send(Cmd::Apply(Action::Inject { dev: pcs[0], port: String::from("eth0"), frame: f }), 0);
    let first = s.eng.trace.next_hop_id();
    let st = s.send(Cmd::StepHop { mask: decode::P_ETH }, 0);
    t.check("step to the next frame", st.is_ok_and(|x| x.new_hops) && s.eng.trace.next_hop_id() == first + 1);
    let idle = s.send(Cmd::StepHop { mask: 0 }, 0);
    t.check("nothing left", idle.is_ok_and(|x| !x.new_hops));
    t.check("speed 4x", {
        let mut q = Inline::new(engine(&p));
        let _ = q.send(Cmd::Realtime { speed_pm: 4000 }, 100);
        q.tick(110).now == 40_000
    });
    t.check("run for", Inline::new(engine(&p)).send(Cmd::RunFor(5 * SEC), 0).is_ok_and(|x| x.now == 5 * SEC));
}
