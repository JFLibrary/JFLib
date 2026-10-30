// keyframetest -- rewinding with keyframes (group keyframes: jfn_sim::keyframe, the driver's
// recording of inputs, doc/sim.rs's past state): snapshots and restores of the campus (the
// state at a moment, LEDs, leases and ARP of then), replays that equal the run byte for byte
// (digest of the whole engine, the frames and decisions they record) at every second of a run
// with console lines, actions, calls and steps of the simulation mode, a replay in slices with
// its progress, the memory budget (keyframes by count and by bytes, the oldest thinned out
// first, the first one gone), the inputs in the history with their times, "Von hier
// fortsetzen" against a run that never went further, and the window's side: the state shown,
// the snapshot of the canvas, a console line in the past, resume with a change of the project
// made later and its undo.  Times are exact virtual times.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use jfn_model::project::Project;
use jfn_sim::keyframe::{digest, footprint, trace_digest};
use jfn_sim::{Action, Budget, Cmd, Engine, Inline, InKind, Led, Mode, Ns, Past, PingOpts, Pos, Report, MS, SEC};

use crate::doc::{edits, Doc};
use crate::selftest::T;

const CAMPUS: &str = include_str!("../tests/fixtures/campus.jfnet");
const LAB: &str = include_str!("../tests/fixtures/lab_window.jfnet");

const R1: u32 = 1;
const MLS1: u32 = 2;
const SRV1: u32 = 3;
const PC1: u32 = 4;
const PC2: u32 = 5;
const PC3: u32 = 6;
const PC4: u32 = 7;
/// the cable PC3 - MLS1 Gi1/0/3
const L_PC3: u32 = 12;

fn campus() -> Project {
    Project::from_text(CAMPUS).unwrap_or_else(|_| Project::new("campus", 5))
}

fn driver(p: &Project, cfg: Budget) -> Inline {
    let mut d = Inline::new(Engine::from_project(p).unwrap_or_else(|_| Engine::new(0)));
    d.rewind_on(cfg);
    d
}

fn cli(d: &mut Inline, dev: u32, line: &str) {
    let _ = d.send(Cmd::Cli { dev, line: String::from(line) }, 0);
}

fn run_for(d: &mut Inline, ns: Ns) {
    let _ = d.send(Cmd::RunFor(ns), 0);
    let _ = d.reports();
}

/// a replay of the driver's present position from its keyframes: (position, digest of the
/// engine, digest of what the replay recorded) and the same of the run
fn replay(d: &Inline) -> Option<((Pos, u64, u64), (Pos, u64, u64))> {
    let pos = d.pos();
    let mut r = d.restore_pos(pos, None)?;
    if !r.run(d.rewind()?, usize::MAX) {
        return None;
    }
    let past = r.finish();
    Some(compare(&past, d))
}

// the past state against the run: position, engine, the recording from where the replay's
// recording begins
fn compare(past: &Past, d: &Inline) -> ((Pos, u64, u64), (Pos, u64, u64)) {
    let (pt, lt) = (&past.eng.trace, &d.eng.trace);
    let h0 = pt.hops().next().map_or(pt.next_hop_id(), |h| h.id);
    let e0 = pt.events().next().map_or(pt.next_event_seq(), |e| e.seq);
    let a = (past.pos, digest(&past.eng), trace_digest(pt, h0..pt.next_hop_id(), e0..pt.next_event_seq()));
    let b = (d.pos(), digest(&d.eng), trace_digest(lt, h0..lt.next_hop_id(), e0..lt.next_event_seq()));
    (a, b)
}

/// the state at moment t, replayed at once
fn at(d: &Inline, t: Ns) -> Option<Past> {
    let mut r = d.restore(t, None)?;
    r.run(d.rewind()?, usize::MAX).then(|| r.finish())
}

pub(crate) fn run(t: &mut T) {
    round_trip(t);
    replays(t);
    past_state(t);
    slices(t);
    budget(t);
    history(t);
    resume(t);
    window(t);
}

// ---------------------------------------------------------------- snapshot and restore

fn round_trip(t: &mut T) {
    let p = campus();
    let mut d = driver(&p, Budget::default());
    let kf = d.rewind().map(|r| r.keyframes()).unwrap_or_default();
    t.eq("first keyframe at the start", kf.iter().map(|k| k.0).collect::<Vec<_>>(), alloc::vec![Pos { t: 0, events: 0, inputs: 0 }]);
    t.check("a keyframe has a size", kf.first().is_some_and(|k| k.1 > 10_000 && k.1 <= footprint(&d.eng)));
    // the start again: the same engine; moment 0 takes the events of moment 0 too
    let start = d.restore_pos(d.pos(), None).map(|mut r| {
        let done = r.run(d.rewind().unwrap_or_else(|| unreachable!()), usize::MAX);
        (done, r.finish())
    });
    t.check("restored at the start", start.as_ref().is_some_and(|(done, a)| *done && digest(&a.eng) == digest(&d.eng) && a.pos == d.pos()));
    let a = at(&d, 0);
    t.eq("moment 0: its events", a.as_ref().map(|a| (a.pos.t, a.pos.events > 0, a.eng.next_time().is_some_and(|x| x > 0))), Some((0, true, true)));
    // ten seconds on: a restore of the present equals the run
    run_for(&mut d, 10 * SEC);
    let r = replay(&d);
    t.check("the present again", r.is_some_and(|(a, b)| a == b));
    // a restore leaves the run alone
    let before = digest(&d.eng);
    let _ = at(&d, 5 * SEC);
    t.eq("the run untouched", digest(&d.eng), before);
    // a moment between two events: everything at or before it
    let a = at(&d, 5 * SEC + 1);
    t.eq("the clock of the moment", a.as_ref().map(|a| a.eng.now()), Some(5 * SEC + 1));
    t.check("no event after it ran", a.as_ref().is_some_and(|a| a.eng.next_time().is_some_and(|x| x > 5 * SEC + 1)));
    // off: nothing is kept, nothing comes back
    let mut off = Inline::new(Engine::from_project(&p).unwrap_or_else(|_| Engine::new(0)));
    run_for(&mut off, SEC);
    t.check("off: no restore", off.rewind().is_none() && off.restore(0, None).is_none());
}

// a realtime run of the campus (16 ms slices) with console lines, actions, a call and steps of
// the simulation mode; every second a replay of the position must equal the run
fn replays(t: &mut T) {
    let p = campus();
    let mut d = driver(&p, Budget { gap_events: 40, ..Budget::default() });
    let _ = d.send(Cmd::Realtime { speed_pm: 1000 }, 0);
    let (mut checked, mut same) = (0, 0);
    let mut wall: u64 = 0;
    while wall < 20_000_000 {
        wall += 16_000;
        let _ = d.tick(wall);
        let _ = d.reports();
        match wall {
            3_008_000 => cli(&mut d, PC1, "ping -c 3 server.lab"),
            4_000_000 => cli(&mut d, R1, "enable"),
            4_016_000 => cli(&mut d, R1, "show ip dhcp binding"),
            6_000_000 => {
                let _ = d.send(Cmd::Apply(Action::Unplug { link: L_PC3, down: true }), wall);
            }
            8_000_000 => {
                let _ = d.send(Cmd::Apply(Action::Unplug { link: L_PC3, down: false }), wall);
            }
            9_008_000 => {
                let _ = d.call("ping", |e| e.ping_host(PC2, "10.10.100.10", PingOpts { count: 2, ..PingOpts::default() }));
            }
            12_000_000 => {
                // the simulation mode: steps between the events of one moment
                let _ = d.send(Cmd::Simulation, wall);
                cli(&mut d, PC4, "ping -c 1 10.10.10.1");
                for _ in 0..5 {
                    let _ = d.send(Cmd::StepEvent, wall);
                    cli(&mut d, MLS1, "show ip arp");
                }
                let _ = d.send(Cmd::StepHop { mask: 0 }, wall);
                let _ = d.send(Cmd::Realtime { speed_pm: 1000 }, wall);
            }
            15_008_000 => cli(&mut d, PC3, "dhclient -v"),
            _ => {}
        }
        if wall % 2_000_000 == 0 || matches!(wall, 3_008_000 | 6_000_000 | 9_008_000 | 15_008_000) {
            checked += 1;
            if let Some((a, b)) = replay(&d) {
                if a == b {
                    same += 1;
                } else {
                    t.check(&format!("replay at {} s: {:?} == {:?}", wall / 1_000_000, a, b), false);
                }
            }
        }
    }
    t.eq("replays equal to the run", (checked, same), (13, 13));
    let rw = d.rewind();
    t.check("keyframes along the run", rw.is_some_and(|r| r.keyframes().len() > 5));
    // the end again from the first keyframe only: the whole run replayed
    let first = rw.and_then(|r| r.keyframes().first().copied()).map(|k| k.0);
    t.eq("first keyframe", first, Some(Pos { t: 0, events: 0, inputs: 0 }));
    let mut long = Inline::new(Engine::from_project(&p).unwrap_or_else(|_| Engine::new(0)));
    long.rewind_on(Budget { gap_events: u64::MAX, ..Budget::default() });
    let _ = long.send(Cmd::RunFor(3 * SEC), 0);
    cli(&mut long, PC1, "ping -c 2 server.lab");
    let _ = long.send(Cmd::RunFor(4 * SEC), 0);
    t.eq("one keyframe only", long.rewind().map(|r| r.keyframes().len()), Some(1));
    t.check("seven seconds replayed from 0", replay(&long).is_some_and(|(a, b)| a == b));
}

// the past differs from the present where it should: leases, ARP, a cable that was down
fn past_state(t: &mut T) {
    let p = campus();
    let mut d = driver(&p, Budget::default());
    run_for(&mut d, 2 * SEC);
    let _ = d.send(Cmd::Apply(Action::Unplug { link: L_PC3, down: true }), 0);
    run_for(&mut d, 2 * SEC);
    let _ = d.send(Cmd::Apply(Action::Unplug { link: L_PC3, down: false }), 0);
    run_for(&mut d, 6 * SEC);
    let eth0 = |e: &Engine, dev: u32| e.ifaces(dev).into_iter().find(|f| f.name == "eth0").and_then(|f| f.addrs.first().copied()).map(|a| a.to_string());
    let led = |e: &Engine| e.view().links.iter().find(|l| l.id == L_PC3).map(|l| l.led);
    let a = at(&d, 500 * MS);
    t.eq("PC1 at 0.5 s: no lease yet", a.as_ref().and_then(|a| eth0(&a.eng, PC1)), None);
    t.eq("PC1 now: its lease", eth0(&d.eng, PC1), Some(String::from("10.10.10.100/24")));
    let arp = |e: &Engine| e.arp_table(MLS1).iter().map(|r| format!("{} {}", r.ip, r.mac.map_or(String::new(), |m| m.to_string()))).collect::<Vec<_>>();
    // then: R1 and the leases it was about to hand out (the relay asks for them), now only R1
    let then = ["10.10.0.1 02:4a:00:00:01:05", "10.10.10.100 ", "10.10.10.101 ", "10.10.20.100 ", "10.10.20.101 "];
    t.eq("MLS1's ARP at 0.5 s", a.as_ref().map(|a| arp(&a.eng)), Some(then.iter().map(|s| String::from(*s)).collect()));
    t.eq("MLS1's ARP now", arp(&d.eng), alloc::vec![String::from("10.10.0.1 02:4a:00:00:01:05")]);
    let down = at(&d, 3 * SEC);
    t.eq("the cable at 3 s", down.as_ref().and_then(|a| led(&a.eng)), Some([Led::Down, Led::Down]));
    t.eq("the cable now", led(&d.eng), Some([Led::Up, Led::Up]));
    t.eq("its clock", down.as_ref().map(|a| a.eng.now()), Some(3 * SEC));
}

// a replay in slices of 500 events: its progress, the same engine in the end
fn slices(t: &mut T) {
    let p = Project::from_text(LAB).unwrap_or_else(|_| Project::new("lab", 1));
    let mut d = driver(&p, Budget { gap_events: u64::MAX, ..Budget::default() });
    run_for(&mut d, 3 * SEC);
    let Some(rw) = d.rewind() else { return t.check("rewind", false) };
    let Some(mut r) = d.restore_pos(d.pos(), None) else { return t.check("restore", false) };
    let (mut calls, mut last, mut rising) = (0, 0, true);
    while !r.run(rw, 500) {
        calls += 1;
        rising &= r.progress() >= last && r.progress() < 1000;
        last = r.progress();
        if calls > 100_000 {
            break;
        }
    }
    t.check("slices: many", calls > 10);
    t.check("slices: progress grows", rising && r.progress() == 1000);
    let past = r.finish();
    let (a, b) = compare(&past, &d);
    t.check("slices: the same engine", a == b);
}

// ---------------------------------------------------------------- the budget

fn kf_times(d: &Inline) -> Vec<Ns> {
    d.rewind().map(|r| r.keyframes().iter().map(|k| k.0.t).collect()).unwrap_or_default()
}

fn budget(t: &mut T) {
    let p = Project::from_text(LAB).unwrap_or_else(|_| Project::new("lab", 1));
    // by count: six keyframes at most, one every 1000 events
    let mut d = driver(&p, Budget { max_count: 6, gap_events: 1_000, ..Budget::default() });
    let mut wall = 0;
    let _ = d.send(Cmd::Realtime { speed_pm: 1000 }, 0);
    while wall < 10_000_000 {
        wall += 16_000;
        let _ = d.tick(wall);
        let _ = d.reports();
    }
    let Some(rw) = d.rewind() else { return t.check("rewind", false) };
    let kf = rw.keyframes();
    t.eq("count: six", kf.len(), 6);
    t.eq("count: made and thinned", (rw.made, rw.thinned), (37, 31));
    t.eq("count: the times", kf_times(&d), alloc::vec![0, 6_096_000_000, 8_544_000_000, 9_376_000_000, 9_616_000_000, 9_904_000_000]);
    t.eq("count: the first stays", kf.first().map(|k| k.0.t), Some(0));
    // the oldest thinned out first: the gaps (in events) shrink towards the present
    let gaps: Vec<u64> = kf.windows(2).map(|w| w[1].0.events - w[0].0.events).collect();
    t.check(&format!("count: gaps shrink {:?}", gaps), gaps.windows(2).all(|g| g[0] >= g[1]));
    t.check("count: bytes", rw.bytes() == kf.iter().map(|k| k.1).sum::<usize>());
    // every moment since the start comes back
    t.check("count: a replay of the present", replay(&d).is_some_and(|(a, b)| a == b));
    t.check("count: at 0.3 s", at(&d, 300 * MS).is_some());
    // by bytes: three keyframes' worth
    let one = kf.first().map_or(0, |k| k.1);
    let mut b = driver(&p, Budget { max_bytes: 3 * one + one / 2, gap_events: 1_000, ..Budget::default() });
    run_for(&mut b, 10 * SEC);
    let rb = b.rewind();
    t.check("bytes: within", rb.is_some_and(|r| r.bytes() <= 3 * one + one / 2));
    t.eq("bytes: the times", kf_times(&b), alloc::vec![0, 10 * SEC]);
    // too little for two: only the newest stays, the start cannot come back
    let mut c = driver(&p, Budget { max_bytes: one, gap_events: 1_000, ..Budget::default() });
    run_for(&mut c, 2 * SEC);
    let _ = c.send(Cmd::StepEvent, 0);
    t.eq("tight: one keyframe", c.rewind().map(|r| r.keyframes().len()), Some(1));
    t.check("tight: the start is gone", c.restore(0, None).is_none() && c.rewind().is_some_and(|r| r.earliest().is_some_and(|e| e > 0)));
    t.check("tight: the inputs before it went", c.rewind().is_some_and(|r| r.inputs().0 > 0 || r.inputs().1.is_empty()));
    t.check("tight: the present comes back", replay(&c).is_some_and(|(a, b)| a == b));
}

// ---------------------------------------------------------------- the history

fn history(t: &mut T) {
    let p = campus();
    let mut d = driver(&p, Budget::default());
    run_for(&mut d, 2 * SEC);
    cli(&mut d, PC1, "ping -c 1 server.lab");
    let _ = d.send(Cmd::Interrupt { dev: PC1 }, 0);
    let _ = d.send(Cmd::Apply(Action::Power { dev: PC4, on: false }), 0);
    let _ = d.call("ping", |e| e.ping_host(PC2, "10.10.100.10", PingOpts::default()));
    let _ = d.send(Cmd::Simulation, 0);
    let _ = d.send(Cmd::StepEvent, 0);
    cli(&mut d, R1, "enable");
    let _ = d.send(Cmd::Seek(SEC), 0);
    let _ = d.send(Cmd::Live, 0);
    let Some(rw) = d.rewind() else { return t.check("rewind", false) };
    let (base, ins) = rw.inputs();
    let rows: Vec<String> = ins.iter().map(|i| format!("{} {} {:?}", crate::workspace::sim::secs(i.at), i.events, i.kind)).collect();
    let want = [
        "2,000 000 000 214 Clock",
        "2,000 000 000 214 Cli(4, \"ping -c 1 server.lab\")",
        "2,000 000 000 214 Interrupt(4)",
        "2,000 000 000 214 Apply(Power { dev: 7, on: false })",
        "2,000 000 000 214 Call(ping)",
        "2,000 004 380 215 Cli(1, \"enable\")",
    ];
    t.eq("inputs", (base, rows), (0, want.iter().map(|s| String::from(*s)).collect()));
    t.eq("input count", rw.input_count(), 6);
    t.check("no input from the view", ins.iter().all(|i| !matches!(i.kind, InKind::Clock) || i.at <= 2 * SEC));
}

// ---------------------------------------------------------------- Von hier fortsetzen

fn resume(t: &mut T) {
    let p = campus();
    // A: on to 9 s with more console lines and a pulled cable after 5 s, then back to 5 s
    let mut a = driver(&p, Budget { gap_events: 30, ..Budget::default() });
    run_for(&mut a, 2 * SEC);
    cli(&mut a, PC1, "ping -c 2 server.lab");
    run_for(&mut a, 3 * SEC);
    let at5 = (a.pos(), digest(&a.eng), a.eng.trace.next_hop_id(), a.eng.trace.next_event_seq());
    run_for(&mut a, SEC);
    cli(&mut a, PC2, "ping -c 2 10.10.100.10");
    let _ = a.send(Cmd::Apply(Action::Unplug { link: L_PC3, down: true }), 0);
    run_for(&mut a, 3 * SEC);
    let later = a.eng.trace.next_hop_id();
    let Some(past) = at(&a, 5 * SEC) else { return t.check("restore 5 s", false) };
    t.eq("the state at 5 s", (past.pos, digest(&past.eng)), (at5.0, at5.1));
    a.resume(past);
    t.eq("resumed: the clock", a.eng.now(), 5 * SEC);
    t.eq("resumed: the recording ends there", (a.eng.trace.next_hop_id(), a.eng.trace.next_event_seq()), (at5.2, at5.3));
    t.check("resumed: frames after 5 s are gone", a.eng.trace.hops().all(|h| h.t_tx <= 5 * SEC) && later > at5.2);
    t.eq("resumed: the view", (a.view(), a.mode()), (None, Mode::Paused));
    t.eq("resumed: inputs", a.rewind().map(|r| r.input_count()), Some(at5.0.inputs));
    t.check("resumed: keyframes up to 5 s", a.rewind().is_some_and(|r| r.keyframes().iter().all(|k| k.0.t <= 5 * SEC)));
    t.eq("resumed: the cable is plugged in", a.eng.view().links.iter().find(|l| l.id == L_PC3).map(|l| l.led), Some([Led::Up, Led::Up]));
    // B: the same run that never went past 5 s; both go on alike
    let mut b = driver(&p, Budget { gap_events: 30, ..Budget::default() });
    run_for(&mut b, 2 * SEC);
    cli(&mut b, PC1, "ping -c 2 server.lab");
    run_for(&mut b, 3 * SEC);
    for d in [&mut a, &mut b] {
        cli(d, PC3, "ping -c 1 server.lab");
        run_for(d, 3 * SEC);
    }
    let all = |d: &Inline| (d.pos(), digest(&d.eng), trace_digest(&d.eng.trace, 1..d.eng.trace.next_hop_id(), 1..d.eng.trace.next_event_seq()));
    t.eq("resumed run equals a run that never went on", all(&a), all(&b));
    t.eq("resumed: the time", a.eng.now(), 8 * SEC);
    // its inputs are those of B
    let ins = |d: &Inline| d.rewind().map(|r| r.inputs().1.iter().map(|i| format!("{} {} {:?}", i.at, i.events, i.kind)).collect::<Vec<_>>()).unwrap_or_default();
    t.eq("resumed: the same inputs", ins(&a), ins(&b));
    // and a replay of the new present works
    t.check("resumed: replay", replay(&a).is_some_and(|(x, y)| x == y));
}

// ---------------------------------------------------------------- the window's side

fn window(t: &mut T) {
    let p = campus();
    let mut doc = Doc::new();
    doc.replace(p.clone(), None);
    let _ = doc.sim.send(Cmd::Simulation);
    let _ = doc.sim.send(Cmd::RunFor(2 * SEC));
    let _ = doc.sim.send(Cmd::Apply(Action::Unplug { link: L_PC3, down: true }));
    let _ = doc.sim.send(Cmd::RunFor(8 * SEC));
    let _ = doc.sim.take_reports();
    // a change of the project at 10 s: PC4 off
    if let Some(e) = edits::device_edit(&doc.proj, PC4, "PC4 ausschalten", |x| x.off = true) {
        doc.apply(e);
    }
    t.check("PC4 off now", doc.sim.engine().node(PC4).is_some_and(|n| !n.power));
    let _ = doc.sim.with_engine("test: no-op", |e| e.now());
    t.check("calls are inputs", doc.sim.rewind().is_some_and(|r| r.inputs().1.iter().any(|i| matches!(i.kind, InKind::Call { what: "test: no-op", .. }))));
    let _ = doc.sim.take_reports();
    let live = digest(doc.sim.engine());
    // the view at 1 s: the devices of then
    let _ = doc.sim.send(Cmd::Seek(SEC));
    t.eq("past: shown", (doc.sim.past(), doc.sim.restoring(), doc.sim.past_gone()), (Some(SEC), None, false));
    t.eq("past: the view's clock", doc.sim.view.now, SEC);
    t.eq("past: the cable then", doc.sim.leds.get(&L_PC3).copied(), Some([Led::Up, Led::Up]));
    t.eq("past: the present's cable", doc.sim.live_view().links.iter().find(|l| l.id == L_PC3).map(|l| l.led), Some([Led::Down, Led::Down]));
    t.check("past: PC4 on then", doc.sim.state().node(PC4).is_some_and(|n| n.power));
    t.check("past: the engine is the present", doc.sim.engine().now() == 10 * SEC && digest(doc.sim.engine()) == live);
    // a console line in the past answers from then and changes nothing
    let out = doc.sim.past_line(MLS1, "show mac address-table");
    t.check("past: show answers", out.as_ref().is_some_and(|o| o.as_ref().is_ok_and(|s| s.contains("Mac Address Table"))));
    let conf = doc.sim.past_line(R1, "enable");
    t.check("past: a line runs", conf.is_some_and(|o| o.is_ok()));
    t.eq("past: R1's prompt stays", doc.sim.state().cli_prompt(R1), String::from("R1>"));
    t.check("past: nothing changed", digest(doc.sim.engine()) == live && doc.sim.take_reports().iter().all(|r| !matches!(r, Report::CliOut { .. })));
    // forward through the recording: the state follows
    let _ = doc.sim.send(Cmd::Seek(3 * SEC));
    t.eq("past at 3 s: the cable then", doc.sim.leds.get(&L_PC3).copied(), Some([Led::Down, Led::Down]));
    let leases = |e: &Engine| e.view().devices.iter().filter(|d| d.ifaces.iter().any(|f| f.name == "eth0" && !f.addrs.is_empty())).count();
    t.eq("past at 3 s: leases then and now", (leases(doc.sim.state()), leases(doc.sim.engine())), (1, 4));
    // an edit brings the view back to the present
    let _ = doc.sim.send(Cmd::Apply(Action::Hostname { dev: SRV1, name: String::from("SRV1") }));
    t.eq("an edit: live again", (doc.sim.view_at(), doc.sim.past()), (None, None));
    t.eq("an edit: the present's cable", doc.sim.leds.get(&L_PC3).copied(), Some([Led::Down, Led::Down]));
    // "Von hier fortsetzen" at 3 s: the project stays, PC4's change takes effect at 3 s
    let _ = doc.sim.send(Cmd::Seek(3 * SEC));
    t.check("resume", doc.sim.resume(&doc.proj));
    t.eq("resume: the clock", doc.sim.engine().now(), 3 * SEC);
    t.check("resume: PC4 off as the project says", doc.sim.engine().node(PC4).is_some_and(|n| !n.power) && doc.proj.device(PC4).is_some_and(|d| d.off));
    t.eq("resume: the cable as at 3 s", doc.sim.leds.get(&L_PC3).copied(), Some([Led::Down, Led::Down]));
    t.eq("resume: the view", (doc.sim.view_at(), doc.sim.past()), (None, None));
    t.check("resume: no frame after 3 s", doc.sim.engine().trace.hops().all(|h| h.t_tx <= 3 * SEC));
    // undo across it works on the project: PC4 on again
    t.check("undo after resume", doc.undo() && doc.sim.engine().node(PC4).is_some_and(|n| n.power) && doc.proj.device(PC4).is_some_and(|d| !d.off));
    t.check("redo after resume", doc.redo() && doc.sim.engine().node(PC4).is_some_and(|n| !n.power));
    // the engine goes on from 3 s
    let _ = doc.sim.send(Cmd::RunFor(SEC));
    t.eq("on from 3 s", doc.sim.engine().now(), 4 * SEC);
    // a moment before the earliest keyframe: the present is shown
    let mut tight = Doc::new();
    tight.replace(p, None);
    tight.sim.set_budget(Budget { max_bytes: 1, gap_events: 10, ..Budget::default() });
    let _ = tight.sim.send(Cmd::Simulation);
    let _ = tight.sim.send(Cmd::RunFor(SEC));
    let _ = tight.sim.with_engine("test: run", |e| {
        e.run_until(2 * SEC, 1_000_000);
    });
    let _ = tight.sim.send(Cmd::StepEvent);
    // the call (the first input: the clock took no sample at 1 s) went with the keyframe before it
    t.eq("tight: the inputs before the newest keyframe went", tight.sim.rewind().map(|r| (r.input_count(), r.inputs().0, r.keyframes().len())), Some((1, 1, 1)));
    let _ = tight.sim.send(Cmd::Seek(500 * MS));
    t.eq("gone: the present shown", (tight.sim.view_at(), tight.sim.past(), tight.sim.past_gone()), (Some(500 * MS), None, true));
    t.check("gone: its state", core::ptr::eq(tight.sim.state(), tight.sim.engine()));
    t.check("gone: no resume", !tight.sim.resume(&tight.proj));
}
