// runner -- a .scn scenario against an engine: its steps at their virtual times, the console
// output of the devices, the expectations, and a report with virtual times only, so that it
// is the same on every machine and every run
//
//   run_scenario(&Project, &Scenario, name) -> Outcome { report, ok, expects, failed, pcap,
//   series }
//
// The steps go through the Inline driver like the user interface's commands (Cmd::Cli,
// Cmd::Interrupt for "^C", RunFor, Apply).  A report line is "    12.000004104  WHO  text":
// the time in seconds, then a device (its console output, a line at the time its newline
// came; typed lines with their prompt), "--" for a step, "ok" or "FEHLER" for an
// expectation.  "expect DEV output contains" looks at what the device printed since its last
// command line of the scenario.  "run until idle" stops after one hour of virtual time when
// something keeps running (an endless ping).  The capture (step "pcap FILE") is made at the
// end: one interface per cable, every frame at the time it was sent; the caller writes it.
// "fault ..." starts a fault of the lab (fault.rs; the line names its number and what it
// does), "heal ..." heals faults by number, by target or all of them; a fault that heals by
// itself after its duration gets a line "Fehler N behoben" with the time it healed.
// "expect DEV svc NAME metric [min|max|mean] M OP V over D" (or "between A and B") checks the
// time series M of the service (series.rs, Engine::series_value) and writes the value it found
// as a decimal; "series FILE" makes the time series a CSV at the end, like the capture, and its
// line names how many series and values it has and the CRC-32 of the file.  The expectations
// are judged by goal.rs (judge, Verdict::report), which the lab's goals in the window share.
// "move DEV X Y" puts a device's box elsewhere (Action::Move: WLAN stations and access points
// look again, wlan.rs).

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use jfn_model::ifname;
use jfn_model::project::Project;
use jfn_model::scenario::{Expect, FaultSpec, HealSpec, Scenario, StepKind};
use netpkt::pcapng::{Writer, LINKTYPE_ETHERNET};

use crate::action::Action;
use crate::driver::{Cmd, Inline, Report};
use crate::fault::Fault;
use crate::goal::{dur, judge, resolve};
use crate::kernel::{Engine, Ns, SEC};

/// how far "run until idle" goes at most
pub const IDLE_CAP: Ns = 3600 * SEC;
const LINKTYPE_C_HDLC: u16 = 104;

/// what a scenario run gives
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outcome {
    pub report: String,
    /// every expectation held and every step could run
    pub ok: bool,
    pub expects: u32,
    pub failed: u32,
    /// the capture the scenario asked for: (file name as written in the scenario, pcapng)
    pub pcap: Option<(String, Vec<u8>)>,
    /// the time series the scenario asked for: (file name as written in the scenario, CSV)
    pub series: Option<(String, String)>,
}

/// every recorded frame as pcapng: one interface per cable, stamped when it was sent
pub fn capture(e: &Engine) -> (Vec<u8>, usize) {
    let mut w = Writer::new("JFNetworker");
    let mut ifs: BTreeMap<u32, u32> = BTreeMap::new();
    let name = |dev: u32, port: u16| match e.node(dev) {
        Some(n) => format!("{} {}", n.name, ifname::short(n.port_name(port))),
        None => format!("{} {}", dev, port),
    };
    let mut n = 0;
    for h in e.trace.hops() {
        let i = match ifs.get(&h.link) {
            Some(i) => *i,
            None => {
                let serial = e.link_rt(h.link).is_some_and(|l| l.cable == jfn_model::CableKind::Serial);
                let i = w.add_interface(&format!("{} \u{2013} {}", name(h.from.0, h.from.1), name(h.to.0, h.to.1)), if serial { LINKTYPE_C_HDLC } else { LINKTYPE_ETHERNET }, 0);
                ifs.insert(h.link, i);
                i
            }
        };
        w.packet(i, h.t_tx, &h.bytes);
        n += 1;
    }
    (w.finish(), n)
}

/// the lines of a report
struct Rep {
    s: String,
    who: usize,
    /// a device's output that has no newline yet: (time of its first byte, text)
    part: BTreeMap<u32, String>,
    /// what a device printed since its last command line
    since: BTreeMap<u32, String>,
    names: BTreeMap<u32, String>,
}

impl Rep {
    fn line(&mut self, t: Ns, who: &str, text: &str) {
        self.s.push_str(&format!("{:>4}.{:09}  {:<w$}  {}\n", t / SEC, t % SEC, who, text, w = self.who));
    }
    fn name(&self, dev: u32) -> String {
        self.names.get(&dev).cloned().unwrap_or_else(|| format!("{}", dev))
    }
    /// console output of a device at `t`
    fn console(&mut self, dev: u32, t: Ns, text: &str) {
        self.since.entry(dev).or_default().push_str(text);
        let mut buf = self.part.remove(&dev).unwrap_or_default();
        buf.push_str(text);
        let name = self.name(dev);
        while let Some(i) = buf.find('\n') {
            let l: String = buf[..i].chars().filter(|&c| c != '\r').collect();
            self.line(t, &name, &l);
            buf.drain(..=i);
        }
        if !buf.is_empty() {
            self.part.insert(dev, buf);
        }
    }
    /// the lines that did not end, at the end of the run
    fn flush(&mut self, t: Ns) {
        let devs: Vec<u32> = self.part.keys().copied().collect();
        for d in devs {
            if let Some(b) = self.part.remove(&d) {
                let n = self.name(d);
                self.line(t, &n, &b);
            }
        }
    }
}

/// what the reports of the engine add to the report
fn drain(d: &mut Inline, r: &mut Rep) {
    for rep in d.reports() {
        match rep {
            Report::CliOut { dev, at, bytes } => {
                let text = String::from_utf8_lossy(&bytes).into_owned();
                r.console(dev, at, &text);
            }
            Report::Startup { dev, .. } => {
                let n = r.name(dev);
                let t = d.eng.now();
                r.line(t, "--", &format!("{}: startup-config gespeichert", n));
            }
            Report::Hostname { dev, name } => {
                let n = r.name(dev);
                let t = d.eng.now();
                r.line(t, "--", &format!("{} heißt jetzt {}", n, name));
            }
            Report::CliState { .. } => {}
            // the console shows its messages already; service logs are not part of a report
            Report::Log { .. } => {}
            Report::Files { dev, at, path, files } => {
                let n = r.name(dev);
                let t = at;
                let text = match files.iter().find(|f| f.path == path) {
                    Some(f) if f.dir => format!("{}: Verzeichnis {} angelegt", n, path),
                    Some(f) => format!("{}: {} gespeichert ({} Bytes)", n, path, f.data.len()),
                    None => format!("{}: {} gelöscht", n, path),
                };
                r.line(t, "--", &text);
            }
        }
    }
}

/// a fault of a scenario with its names resolved
fn fault_of(p: &Project, e: &Engine, f: &FaultSpec) -> Result<Fault, String> {
    let dev = |n: &str| resolve(p, e, n).ok_or_else(|| format!("das Gerät {} gibt es nicht", n));
    let link = |n: &str, port: &str| -> Result<u32, String> {
        let d = dev(n)?;
        p.link_at(d, port).map(|l| l.id).ok_or_else(|| format!("an {} {} steckt kein Kabel", n, port))
    };
    let devs = |v: &[String]| -> Result<Vec<u32>, String> { v.iter().map(|n| dev(n)).collect() };
    Ok(match f {
        FaultSpec::Link { dev: d, port } => Fault::Unplug { link: link(d, port)? },
        FaultSpec::Impair { dev: d, port, imp } => Fault::Impair {
            link: link(d, port)?,
            delay: imp.delay_ns.unwrap_or(0),
            jitter: imp.jitter_ns.unwrap_or(0),
            loss_ppm: imp.loss_ppm.unwrap_or(0),
            kbps: imp.bps.map(|b| (b / 1000).clamp(1, u32::MAX as u64) as u32),
        },
        FaultSpec::Power { dev: d } => Fault::Power { dev: dev(d)? },
        FaultSpec::Stop { dev: d, svc } => Fault::Stop { dev: dev(d)?, svc: svc.clone() },
        FaultSpec::Disk { dev: d, delay_ns, factor_pm } => Fault::SlowDisk { dev: dev(d)?, extra: *delay_ns, factor_pm: *factor_pm },
        FaultSpec::Partition { a, b } => Fault::Partition { a: devs(a)?, b: devs(b)? },
    })
}

/// the same kind of fault on the same target
fn same_target(a: &Fault, b: &Fault) -> bool {
    match (a, b) {
        (Fault::Unplug { link: x }, Fault::Unplug { link: y }) => x == y,
        (Fault::Impair { link: x, .. }, Fault::Impair { link: y, .. }) => x == y,
        (Fault::Power { dev: x }, Fault::Power { dev: y }) => x == y,
        (Fault::Stop { dev: x, svc: s }, Fault::Stop { dev: y, svc: t }) => x == y && s.eq_ignore_ascii_case(t),
        (Fault::SlowDisk { dev: x, .. }, Fault::SlowDisk { dev: y, .. }) => x == y,
        (Fault::Partition { .. }, Fault::Partition { .. }) => true,
        _ => false,
    }
}

/// lines for the faults that healed by themselves since the last call
fn heal_lines(e: &Engine, r: &mut Rep, shown: &mut BTreeSet<u32>) {
    for m in e.faults() {
        if let Some(t) = m.healed
            && shown.insert(m.id)
        {
            r.line(t, "--", &format!("Fehler {} behoben: {}", m.id, m.text));
        }
    }
}

/// runs a scenario against a project (`pcap`: capture also without a "pcap" step)
pub fn run_scenario(p: &Project, sc: &Scenario, name: &str, pcap: bool) -> Outcome {
    let force_pcap = pcap;
    let mut proj = p.clone();
    if let Some(s) = sc.seed {
        proj.seed = s;
    }
    let mut names = BTreeMap::new();
    for dv in &proj.devices {
        names.insert(dv.id, dv.name.clone());
    }
    let who = names.values().map(|n| n.chars().count()).max().unwrap_or(0).max(6);
    let mut r = Rep { s: String::new(), who, part: BTreeMap::new(), since: BTreeMap::new(), names };
    r.s.push_str(&format!("# JFNetworker-Szenario {}: \u{201e}{}\u{201c}, {} Geräte, {} Verbindungen, Seed {}\n", name, proj.title, proj.devices.len(), proj.links.len(), proj.seed));
    let eng = match Engine::from_project(&proj) {
        Ok(e) => e,
        Err(e) => {
            r.s.push_str(&format!("# Das Projekt läuft nicht: {}\n", e));
            return Outcome { report: r.s, ok: false, expects: 0, failed: 0, pcap: None, series: None };
        }
    };
    let mut d = Inline::new(eng);
    drain(&mut d, &mut r);
    let (mut expects, mut failed, mut ok) = (0u32, 0u32, true);
    let mut pcap: Option<String> = None;
    let mut series: Option<String> = None;
    let mut healed: BTreeSet<u32> = BTreeSet::new();
    for step in &sc.steps {
        if let Some(t) = step.at {
            let now = d.eng.now();
            if t > now {
                let _ = d.send(Cmd::RunFor(t - now), 0);
                drain(&mut d, &mut r);
                heal_lines(&d.eng, &mut r, &mut healed);
            }
        }
        let now = d.eng.now();
        let dev_of = |n: &str, d: &Inline| resolve(&proj, &d.eng, n);
        match &step.kind {
            StepKind::Cli { dev, cmd } => {
                let Some(id) = dev_of(dev, &d) else {
                    r.line(now, "FEHLER", &format!("Zeile {}: das Gerät {} gibt es nicht", step.line, dev));
                    ok = false;
                    continue;
                };
                if cmd == "^C" {
                    r.console(id, now, "^C");
                    if let Err(e) = d.send(Cmd::Interrupt { dev: id }, 0) {
                        r.line(now, "FEHLER", &format!("Zeile {}: {}", step.line, e));
                        ok = false;
                    }
                } else {
                    if !d.eng.cli_busy(id) {
                        let echo = format!("{}{}\n", d.eng.cli_prompt(id), cmd);
                        r.console(id, now, &echo);
                    }
                    r.since.insert(id, String::new());
                    if let Err(e) = d.send(Cmd::Cli { dev: id, line: cmd.clone() }, 0) {
                        r.line(now, "FEHLER", &format!("Zeile {}: {}", step.line, e));
                        ok = false;
                    }
                }
            }
            StepKind::Run(t) => {
                r.line(now, "--", &format!("run {}", dur(*t)));
                let _ = d.send(Cmd::RunFor(*t), 0);
            }
            StepKind::RunIdle => {
                r.line(now, "--", "run until idle");
                let end = now + IDLE_CAP;
                let mut idle = true;
                while let Some(t) = d.eng.next_time() {
                    if t > end {
                        idle = false;
                        break;
                    }
                    d.eng.step();
                }
                if !idle {
                    d.eng.run_until(end, usize::MAX);
                    drain(&mut d, &mut r);
                    let t = d.eng.now();
                    r.line(t, "--", "nach einer Stunde virtueller Zeit noch nicht zur Ruhe gekommen");
                }
            }
            StepKind::Link { dev, port, up } => {
                let Some(link) = dev_of(dev, &d).and_then(|id| proj.link_at(id, port)).map(|l| l.id) else {
                    r.line(now, "FEHLER", &format!("Zeile {}: an {} {} steckt kein Kabel", step.line, dev, port));
                    ok = false;
                    continue;
                };
                r.line(now, "--", &format!("link {} {} {}", if *up { "up" } else { "down" }, dev, port));
                let _ = d.send(Cmd::Apply(Action::Unplug { link, down: !*up }), 0);
            }
            StepKind::Impair { dev, port, imp } => {
                let Some(link) = dev_of(dev, &d).and_then(|id| proj.link_at(id, port)).map(|l| l.id) else {
                    r.line(now, "FEHLER", &format!("Zeile {}: an {} {} steckt kein Kabel", step.line, dev, port));
                    ok = false;
                    continue;
                };
                let mut what = Vec::new();
                if let Some(l) = imp.loss_ppm {
                    what.push(format!("loss {}ppm", l));
                }
                if let Some(x) = imp.delay_ns {
                    what.push(format!("delay {}", dur(x)));
                }
                if let Some(x) = imp.jitter_ns {
                    what.push(format!("jitter {}", dur(x)));
                }
                if let Some(x) = imp.bps {
                    what.push(format!("rate {}", x));
                }
                r.line(now, "--", &format!("impair {} {} {}", dev, port, what.join(" ")));
                let kbps = imp.bps.map(|b| (b / 1000).clamp(1, u32::MAX as u64) as u32);
                let _ = d.send(Cmd::Apply(Action::Impair { link, kbps, delay_ns: imp.delay_ns, jitter_ns: imp.jitter_ns, loss_ppm: imp.loss_ppm }), 0);
            }
            StepKind::Power { dev, on } => match dev_of(dev, &d) {
                Some(id) => {
                    r.line(now, "--", &format!("power {} {}", if *on { "on" } else { "off" }, dev));
                    let _ = d.send(Cmd::Apply(Action::Power { dev: id, on: *on }), 0);
                }
                None => {
                    r.line(now, "FEHLER", &format!("Zeile {}: das Gerät {} gibt es nicht", step.line, dev));
                    ok = false;
                }
            },
            StepKind::Expect(x) => {
                drain(&mut d, &mut r);
                expects += 1;
                let (held, text) = expect(&proj, &d.eng, &r, x);
                let t = d.eng.now();
                r.line(t, if held { "ok" } else { "FEHLER" }, &text);
                if !held {
                    failed += 1;
                    ok = false;
                }
            }
            StepKind::Pcap(f) => pcap = Some(f.clone()),
            StepKind::Series(f) => series = Some(f.clone()),
            StepKind::Print(t) => r.line(now, "--", t),
            // a device moves (WLAN looks again: wlan.rs)
            StepKind::Move { dev, x, y } => match dev_of(dev, &d) {
                Some(id) => {
                    r.line(now, "--", &format!("move {} {} {}", dev, x, y));
                    if let Err(e) = d.send(Cmd::Apply(Action::Move { dev: id, x: *x, y: *y }), 0) {
                        r.line(now, "FEHLER", &format!("Zeile {}: {}", step.line, e));
                        ok = false;
                    }
                }
                None => {
                    r.line(now, "FEHLER", &format!("Zeile {}: das Gerät {} gibt es nicht", step.line, dev));
                    ok = false;
                }
            },
            StepKind::Fault { fault, dur } => match fault_of(&proj, &d.eng, fault) {
                Ok(f) => {
                    let id = d.eng.next_fault_id();
                    match d.send(Cmd::Apply(Action::Fault { fault: f, dur: *dur }), 0) {
                        Ok(_) => {
                            let text = d.eng.faults().iter().find(|m| m.id == id).map(|m| m.text.clone()).unwrap_or_default();
                            let span = dur.map(|x| format!(" (für {})", self::dur(x))).unwrap_or_default();
                            r.line(now, "--", &format!("fault {}: {}{}", id, text, span));
                        }
                        Err(e) => {
                            r.line(now, "FEHLER", &format!("Zeile {}: {}", step.line, e));
                            ok = false;
                        }
                    }
                }
                Err(e) => {
                    r.line(now, "FEHLER", &format!("Zeile {}: {}", step.line, e));
                    ok = false;
                }
            },
            StepKind::Heal(h) => {
                let ids: Result<Vec<u32>, String> = match h {
                    HealSpec::All => Ok(d.eng.active_faults().map(|m| m.id).collect()),
                    HealSpec::Id(n) => Ok(alloc::vec![*n]),
                    HealSpec::Like(spec) => fault_of(&proj, &d.eng, spec).map(|f| d.eng.active_faults().filter(|m| same_target(&m.fault, &f)).map(|m| m.id).collect()),
                };
                match ids {
                    Ok(ids) if ids.is_empty() => r.line(now, "--", "heal: kein aktiver Fehler passt"),
                    Ok(ids) => {
                        for id in ids {
                            let text = d.eng.faults().iter().find(|m| m.id == id).map(|m| m.text.clone()).unwrap_or_default();
                            match d.send(Cmd::Apply(Action::Heal { id }), 0) {
                                Ok(_) => {
                                    healed.insert(id);
                                    r.line(now, "--", &format!("heal {}: {}", id, text));
                                }
                                Err(e) => {
                                    r.line(now, "FEHLER", &format!("Zeile {}: {}", step.line, e));
                                    ok = false;
                                }
                            }
                        }
                    }
                    Err(e) => {
                        r.line(now, "FEHLER", &format!("Zeile {}: {}", step.line, e));
                        ok = false;
                    }
                }
            }
        }
        drain(&mut d, &mut r);
        heal_lines(&d.eng, &mut r, &mut healed);
    }
    let end = d.eng.now();
    r.flush(end);
    let cap = pcap.or(if force_pcap { Some(String::new()) } else { None }).map(|f| {
        let (bytes, n) = capture(&d.eng);
        if !f.is_empty() {
            r.line(end, "--", &format!("pcap {}: {} Frames", f, n));
        }
        (f, bytes)
    });
    let csv = series.map(|f| {
        let text = d.eng.series_csv();
        let (n, v) = d.eng.series_size();
        r.line(end, "--", &format!("series {}: {} Reihen, {} Werte, CRC-32 {:08x}", f, n, v, netpkt::checksum::crc32(text.as_bytes())));
        (f, text)
    });
    r.s.push_str(&format!("# {} von {} Erwartungen erfüllt{}\n", expects - failed, expects, if ok { "" } else { ", Szenario fehlgeschlagen" }));
    Outcome { report: r.s, ok, expects, failed, pcap: cap, series: csv }
}

/// whether an expectation holds, and its line (goal.rs judges it; "output contains" looks at
/// what the device printed since its last command line)
fn expect(p: &Project, e: &Engine, r: &Rep, x: &Expect) -> (bool, String) {
    let v = judge(p, e, x, &|id| r.since.get(&id).cloned().unwrap_or_default());
    (v.held, v.report())
}
