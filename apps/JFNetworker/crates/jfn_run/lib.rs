// jfn_run -- JFNetworker without a window, for the GUI program (--scenario) and jfn-sim: read
// the files, run, write the report and the capture
//
//   gui_args(&args) -> Option<i32>   the GUI program's early exit: "--scenario S [PROJECT]
//                                    [--pcap F] [--report F] [--golden F] [--series F]" runs
//                                    and ends
//   main(&args) -> i32               jfn-sim: run PROJECT [--scenario S] [--pcap F] [--report F]
//                                    [--golden F] [--series F] | check PROJECT | decode HEX |
//                                    gen N [--out F]
//   generate(&args) -> i32           jfn-sim gen: a synthetic project (jfn_model::synth)
//   rewind(&args) -> i32             jfn-sim rewind PROJECT [SECONDS] [--traffic K]: the
//                                    keyframes of a run (K1) and how long restores take (times
//                                    of this machine)
//
// Exit codes: 0 everything held, 1 an expectation failed, the project has errors or the report
// differs from the golden file, 2 a file could not be read or the command line is wrong.  The
// report goes to stdout and with --report to a file, byte for byte the same on every machine;
// --golden compares it with a stored one (CR LF there counts as LF).  With the app's
// PERMISSIONS (fs.user) the files must be named on the command line: a "pcap FILE" step of a
// scenario writes only when FILE is granted that way, else use --pcap.  --series F writes the
// time series as CSV to F, for the scenario's "series FILE" step or, without one, as if the
// scenario ended with "series F".
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use jfn_model::project::Project;
use jfn_model::scenario::{self, Scenario, Step, StepKind};

/// what a run is given
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Opts {
    pub project: Option<String>,
    pub scenario: Option<String>,
    pub pcap: Option<String>,
    pub report: Option<String>,
    pub golden: Option<String>,
    pub series: Option<String>,
}

fn say(s: &str) {
    plat::write_stdout(s.as_bytes());
}

fn complain(s: &str) {
    plat::write_stderr(s.as_bytes());
}

/// the options of a run; values of options the GUI program has for itself are skipped
pub fn parse_opts(args: &[String]) -> Result<Opts, String> {
    let mut o = Opts::default();
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        let (key, inline) = match a.split_once('=') {
            Some((k, v)) if k.starts_with("--") => (k, Some(String::from(v))),
            _ => (a, None),
        };
        let mut value = || -> Result<String, String> {
            if let Some(v) = inline.clone() {
                return Ok(v);
            }
            i += 1;
            args.get(i).cloned().ok_or_else(|| format!("{} braucht einen Wert.", key))
        };
        match key {
            "--scenario" => o.scenario = Some(value()?),
            "--pcap" => o.pcap = Some(value()?),
            "--report" => o.report = Some(value()?),
            "--golden" => o.golden = Some(value()?),
            "--series" => o.series = Some(value()?),
            "--script" | "--shot" | "--style" | "--lang" => {
                value()?;
            }
            k if k.starts_with('-') => {}
            p => {
                if o.project.is_some() {
                    return Err(format!("Zwei Projekte angegeben: {} und {}.", o.project.as_deref().unwrap_or(""), p));
                }
                o.project = Some(String::from(p));
            }
        }
        i += 1;
    }
    Ok(o)
}

/// a text file (a UTF-8 byte order mark at the start is dropped, as editors write one)
fn read_text(path: &str) -> Result<String, String> {
    let b = plat::read_file(path).ok_or_else(|| format!("{} lässt sich nicht lesen.", path))?;
    let s = String::from_utf8(b).map_err(|_| format!("{} ist kein UTF-8-Text.", path))?;
    Ok(match s.strip_prefix('\u{feff}') {
        Some(r) => String::from(r),
        None => s,
    })
}

fn base_name(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

fn load(path: &str) -> Result<Project, String> {
    let text = read_text(path)?;
    Project::from_text(&text).map_err(|e| format!("{}: {}", path, e))
}

/// the GUI program's early exit for headless scenario runs
pub fn gui_args(args: &[String]) -> Option<i32> {
    if !args.iter().any(|a| a == "--scenario" || a.starts_with("--scenario=")) {
        return None;
    }
    Some(match parse_opts(args.get(1..).unwrap_or(&[])) {
        Ok(o) => run(&o),
        Err(e) => {
            complain(&format!("{}\n", e));
            2
        }
    })
}

/// a scenario (or none) against a project
pub fn run(o: &Opts) -> i32 {
    let Some(pp) = o.project.as_deref() else {
        complain("Kein Projekt angegeben (die .jfnet-Datei vor oder nach --scenario).\n");
        return 2;
    };
    let proj = match load(pp) {
        Ok(p) => p,
        Err(e) => {
            complain(&format!("{}\n", e));
            return 2;
        }
    };
    let mut sc = match o.scenario.as_deref() {
        Some(s) => {
            let text = match read_text(s) {
                Ok(t) => t,
                Err(e) => {
                    complain(&format!("{}\n", e));
                    return 2;
                }
            };
            match scenario::parse(&text) {
                Ok(sc) => sc,
                Err(errs) => {
                    for e in errs {
                        complain(&format!("{}:{}: {}\n", s, e.line, e.text));
                    }
                    return 2;
                }
            }
        }
        None => Scenario::default(),
    };
    if let Some(f) = o.series.as_deref()
        && !sc.steps.iter().any(|s| matches!(s.kind, StepKind::Series(_)))
    {
        sc.steps.push(Step { line: 0, at: None, kind: StepKind::Series(String::from(f)) });
    }
    let name = base_name(o.scenario.as_deref().unwrap_or(pp));
    let out = jfn_sim::run_scenario(&proj, &sc, name, o.pcap.is_some());
    say(&out.report);
    let mut code = if out.ok { 0 } else { 1 };
    if let Some(f) = o.report.as_deref()
        && !plat::write_file(f, out.report.as_bytes())
    {
        complain(&format!("{} lässt sich nicht schreiben.\n", f));
        code = 2;
    }
    if let Some((file, bytes)) = &out.pcap {
        let target = o.pcap.as_deref().unwrap_or(file);
        if !plat::write_file(target, bytes) {
            complain(&format!("pcap: {} lässt sich nicht schreiben (mit --pcap auf der Befehlszeile nennen).\n", target));
            code = code.max(1);
        }
    }
    if let Some((file, csv)) = &out.series {
        let target = o.series.as_deref().unwrap_or(file);
        if !plat::write_file(target, csv.as_bytes()) {
            complain(&format!("series: {} lässt sich nicht schreiben (mit --series auf der Befehlszeile nennen).\n", target));
            code = code.max(1);
        }
    }
    if let Some(g) = o.golden.as_deref() {
        match read_text(g) {
            Ok(want) => {
                let want = want.replace("\r\n", "\n");
                if want != out.report {
                    let (wl, gl): (Vec<&str>, Vec<&str>) = (want.lines().collect(), out.report.lines().collect());
                    let i = wl.iter().zip(gl.iter()).position(|(a, b)| a != b).unwrap_or(wl.len().min(gl.len()));
                    complain(&format!("FAIL scenario: der Bericht weicht von {} ab (Zeile {}):\n- {}\n+ {}\n", g, i + 1, wl.get(i).unwrap_or(&""), gl.get(i).unwrap_or(&"")));
                    code = code.max(1);
                }
            }
            Err(e) => {
                complain(&format!("{}\n", e));
                code = 2;
            }
        }
    }
    if code == 1 && !out.ok {
        complain(&format!("FAIL scenario: {} von {} Erwartungen nicht erfüllt\n", out.failed, out.expects));
    }
    code
}

/// consistency of a project and of its startup configurations
pub fn check(path: &str) -> i32 {
    let proj = match load(path) {
        Ok(p) => p,
        Err(e) => {
            complain(&format!("{}\n", e));
            return 2;
        }
    };
    let mut errors = 0;
    let name = |id: Option<u32>| id.and_then(|i| proj.device(i)).map(|d| d.name.clone());
    for i in proj.check() {
        let who = name(i.dev).map(|n| format!("{}: ", n)).unwrap_or_default();
        say(&format!("{}{}{}\n", if i.error { "Fehler: " } else { "Warnung: " }, who, i.text));
        if i.error {
            errors += 1;
        }
    }
    match jfn_sim::Engine::from_project(&proj) {
        Ok(e) => {
            for d in &proj.devices {
                for b in e.boot_errors(d.id) {
                    say(&format!("Fehler: {}, startup-config Zeile {}: {}\n    {}\n", d.name, b.line, b.text, b.msg));
                    errors += 1;
                }
            }
        }
        Err(e) => {
            say(&format!("Fehler: das Projekt läuft nicht: {}\n", e));
            errors += 1;
        }
    }
    say(&format!("{}: {} Geräte, {} Verbindungen, {}\n", base_name(path), proj.devices.len(), proj.links.len(), if errors == 0 { String::from("in Ordnung") } else { format!("{} Fehler", errors) }));
    if errors == 0 { 0 } else { 1 }
}

/// a frame in hex as a capture tool shows its layers
pub fn decode(hex: &str) -> i32 {
    let digits: String = hex.chars().filter(|c| !c.is_whitespace() && *c != ':' && *c != '-' && *c != '.').collect();
    let digits = digits.strip_prefix("0x").unwrap_or(&digits);
    if digits.is_empty() || digits.len() % 2 != 0 || !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
        complain("decode braucht die Bytes eines Frames als Hex (gerade Anzahl Ziffern).\n");
        return 2;
    }
    let bytes: Vec<u8> = (0..digits.len() / 2).filter_map(|i| u8::from_str_radix(&digits[2 * i..2 * i + 2], 16).ok()).collect();
    let mut s = String::new();
    for l in netpkt::decode::layers(&bytes) {
        s.push_str(&format!("{}\n", l.title));
        for f in &l.fields {
            s.push_str(&format!("    {}: {}\n", f.name, f.value));
        }
        for p in &l.problems {
            s.push_str(&format!("    ! {}\n", p));
        }
    }
    say(&s);
    0
}

/// a synthetic project of n devices (jfn_model::synth) as a .jfnet file, else on stdout
pub fn generate(args: &[String]) -> i32 {
    let Some(n) = args.first().and_then(|a| a.trim().parse::<usize>().ok()) else {
        complain(USAGE);
        return 2;
    };
    let p = jfn_model::synth::topology(n);
    let text = p.to_text();
    match args.iter().position(|a| a == "--out").and_then(|i| args.get(i + 1)) {
        Some(f) => {
            if !plat::write_file(f, text.as_bytes()) {
                complain(&format!("{} lässt sich nicht schreiben.\n", f));
                return 2;
            }
            say(&format!("{}: {} Geräte, {} Verbindungen\n", base_name(f), p.devices.len(), p.links.len()));
        }
        None => say(&text),
    }
    0
}

const USAGE: &str = "jfn-sim -- JFNetworker ohne Fenster\n\n  jfn-sim run PROJEKT [--scenario S] [--pcap F] [--report F] [--golden F] [--series F]\n  jfn-sim check PROJEKT\n  jfn-sim decode HEX\n  jfn-sim gen N [--out DATEI]\n  jfn-sim rewind PROJEKT [SEKUNDEN] [--traffic K]\n";

/// jfn-sim's command line (args[0] is the program)
pub fn main(args: &[String]) -> i32 {
    match args.get(1).map(String::as_str) {
        Some("run") => match parse_opts(&args[2..]) {
            Ok(o) => run(&o),
            Err(e) => {
                complain(&format!("{}\n", e));
                2
            }
        },
        Some("check") => match args.get(2) {
            Some(p) => check(p),
            None => {
                complain(USAGE);
                2
            }
        },
        Some("decode") if args.len() > 2 => decode(&args[2..].join(" ")),
        Some("gen") => generate(&args[2..]),
        Some("rewind") => rewind(&args[2..]),
        Some("help" | "--help" | "-h") => {
            say(USAGE);
            0
        }
        _ => {
            complain(USAGE);
            2
        }
    }
}

// ---------------------------------------------------------------- rewinding (K1)

/// jfn-sim rewind PROJEKT [SEKUNDEN] [--traffic K]: what rewinding costs for a project -- it
/// runs SEKUNDEN (default 60) of virtual time in slices of 16 ms, once without and once with
/// keyframes (the window's budget, jfn_sim::Budget::default()), K pings at the start; then the
/// keyframes (how many, how big) and ten moments restored (time, events replayed).  Times are
/// of this machine, so the output differs from run to run.
pub fn rewind(args: &[String]) -> i32 {
    use jfn_sim::{Budget, Cmd, Engine, Inline};
    let Some(path) = args.first() else {
        complain(USAGE);
        return 2;
    };
    let secs = args.get(1).and_then(|a| a.parse::<u64>().ok()).unwrap_or(60).max(1);
    let traffic = args.iter().position(|a| a == "--traffic").and_then(|i| args.get(i + 1)).and_then(|v| v.parse::<usize>().ok()).unwrap_or(0);
    let proj = match load(path) {
        Ok(p) => p,
        Err(e) => {
            complain(&format!("{}\n", e));
            return 2;
        }
    };
    let ms = |us: u64| format!("{},{:02}", us / 1000, us % 1000 / 10);
    let kib = |b: usize| format!("{} KiB", b.div_ceil(1024));
    // the same run twice: without keyframes, then with them
    let mut runs = Vec::new();
    for on in [false, true] {
        let mut d = Inline::new(Engine::from_project(&proj).unwrap_or_else(|_| Engine::new(proj.seed)));
        let t0 = plat::time_us();
        if on {
            d.rewind_on(Budget::default());
        }
        if traffic > 0 {
            d.call("traffic", move |e| e.ping_many(traffic));
        }
        let _ = d.send(Cmd::Realtime { speed_pm: 1000 }, 0);
        let mut wall = 0;
        while wall < secs * 1_000_000 {
            wall += 16_000;
            let _ = d.tick(wall);
            let _ = d.reports();
        }
        runs.push((d, plat::time_us() - t0));
    }
    let Some((d, on_us)) = runs.pop() else { return 2 };
    let off_us = runs.pop().map_or(0, |r| r.1);
    let Some(rw) = d.rewind() else { return 2 };
    let kf = rw.keyframes();
    let (min, max) = (kf.iter().map(|k| k.1).min().unwrap_or(0), kf.iter().map(|k| k.1).max().unwrap_or(0));
    say(&format!("{}: {} s virtuelle Zeit, {} Ereignisse; ohne Keyframes {} ms, mit {} ms\n", base_name(path), secs, d.eng.events_done(), ms(off_us), ms(on_us)));
    say(&format!("Keyframes: {} (gemacht {}, ausgedünnt {}), zusammen {}, je {} bis {}\n", kf.len(), rw.made, rw.thinned, kib(rw.bytes()), kib(min), kib(max)));
    // what a keyframe of the present would hold, by part (the recording left out)
    let f = jfn_sim::Footprint::of(&d.eng);
    let rec = d.eng.trace.hops().len() * (core::mem::size_of::<jfn_sim::Hop>() + 16);
    say(&format!("  jetzt: Geräte {}, davon Logs {}, Dienste {}, TCP {}; Kabel {}, Zeitreihen {}, Ereignisse {}, Rest {}\n", kib(f.devices + f.logs + f.services + f.tcp), kib(f.logs), kib(f.services), kib(f.tcp), kib(f.cables), kib(f.series), kib(f.queue), kib(f.rest.saturating_sub(rec))));
    let times: Vec<String> = kf.iter().map(|k| format!("{},{:03}", k.0.t / jfn_sim::SEC, k.0.t % jfn_sim::SEC / jfn_sim::MS)).collect();
    say(&format!("  bei t = {} s\n", times.join("; ")));
    // ten moments, from the start to the end
    let end = d.eng.now();
    let mut took = Vec::new();
    for i in 0..10u64 {
        let t = end / 10 * i + end / 20;
        let t0 = plat::time_us();
        let Some(mut r) = d.restore(t, None) else { continue };
        let from = kf.iter().rev().find(|k| k.0.t <= t).map_or(0, |k| k.0.events);
        let _ = r.run(rw, usize::MAX);
        let past = r.finish();
        let us = plat::time_us() - t0;
        say(&format!("  t = {:>7} s: {} ms, {} Ereignisse nachgespielt\n", format!("{},{:03}", t / jfn_sim::SEC, t % jfn_sim::SEC / jfn_sim::MS), ms(us), past.pos.events - from));
        took.push(us);
    }
    took.sort_unstable();
    if let (Some(a), Some(b)) = (took.first(), took.last()) {
        say(&format!("Wiederherstellen: {} / {} / {} ms (min / Median / max)\n", ms(*a), ms(took[took.len() / 2]), ms(*b)));
    }
    0
}
