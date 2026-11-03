// jfn_run -- JFNetworker without a window, for the GUI program (--scenario) and jfn-sim: read
// the files, run, write the report and the capture
//
//   gui_args(&args) -> Option<i32>   the GUI program's early exit: "--scenario S [PROJECT]
//                                    [--pcap F] [--report F] [--golden F] [--series F]" or "--sweep
//                                    FILE [PROJECT] [...]" runs and ends
//   main(&args) -> i32               jfn-sim: run PROJECT [--scenario S] [--pcap F] [--report F]
//                                    [--golden F] [--series F] | check PROJECT | course DIR |
//                                    decode HEX | gen N [--out F] | sweep FILE [PROJECT]
//                                    [--threads N] [--csv F] [--detail F] [--report F] [--golden F]
//                                    [--name N] [--timing] | course-install SOURCE COURSE --into DIR |
//                                    course-list SOURCE | course-remove COURSE --into DIR (install.rs)
//   install                          installing courses, from the web or a folder, for the window and
//                                    for jfn-sim (install.rs)
//   sweep(&args) -> i32              jfn-sim sweep (SA2): an experiment (a .jfnex file with its project,
//                                    or a project with its section) at full speed: the table goes to
//                                    stdout, the same bytes for any number of threads
//   run_sweep(&plan, threads) -> Sweep   jfn_sim's experiment on worker threads: a queue of boots and
//                                    of levels, levels first, so that at most as many warm engines
//                                    are held as there are threads; results by their place, so
//                                    the order the threads finish in leaves no trace
//   SweepJob::start(exp, project, name, threads) -> Option<SweepJob>   the same in the background (SA3): the
//                                    plan is made and run on threads of their own, from these copies;
//                                    done() / total() the runs finished, cancel(), finished(), take() ->
//                                    SweepOut (Done, Refused, Cancelled: no half result comes back)
//   sweeps_pending() -> bool         a job of any window is under way (ui.sync_hook_set)
//   run_sweep_ctl(&plan, threads, &ctl) -> Option<Sweep>   run_sweep with SweepCtl: counts the runs,
//                                    None when it was cancelled
//   check_project(&project) -> (lines, errors)   what `check` finds, without printing it
//   check_course(dir) -> (lines, errors)   jfn-sim course: a course's folder (jfn_model::academy):
//                                    the file, its lessons in every language with their pictures and
//                                    links, its labs with `check`'s findings; a line per finding,
//                                    errors are what keeps the course from being offered
//   course_findings(dir, id)         the same as records, for a folder not named after the course
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

pub mod install;

use alloc::boxed::Box;
use alloc::collections::VecDeque;
use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};

use jfn_model::experiment::Experiment;
use jfn_model::project::Project;
use jfn_model::scenario::{self, Scenario, Step, StepKind};
use jfn_model::msg::Msg;
use jfn_sim::experiment::{Fig, Plan, Status, Stop, Sweep};
use jfn_sim::Engine;
use plat::sync::Mutex;

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
            "--script" | "--shot" | "--style" | "--lang" | "--academy-dir" | "--academy-folder" | "--sweep" | "--threads" | "--csv" | "--detail" | "--name" => {
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
    if args.iter().any(|a| a == "--sweep" || a.starts_with("--sweep=")) {
        return Some(sweep(args.get(1..).unwrap_or(&[])));
    }
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
    let (lines, errors) = check_project(&proj);
    for l in &lines {
        say(l);
    }
    say(&format!("{}: {} Geräte, {} Verbindungen, {}\n", base_name(path), proj.devices.len(), proj.links.len(), if errors == 0 { String::from("in Ordnung") } else { format!("{} Fehler", errors) }));
    if errors == 0 { 0 } else { 1 }
}

/// what `check` finds in a project: its lines, how many of them are errors
pub fn check_project(proj: &Project) -> (Vec<String>, usize) {
    let mut lines = Vec::new();
    let mut errors = 0;
    let name = |id: Option<u32>| id.and_then(|i| proj.device(i)).map(|d| d.name.clone());
    for i in proj.check() {
        let who = name(i.dev).map(|n| format!("{}: ", n)).unwrap_or_default();
        lines.push(format!("{}{}{}\n", if i.error { "Fehler: " } else { "Warnung: " }, who, i.text));
        if i.error {
            errors += 1;
        }
    }
    match jfn_sim::Engine::from_project(proj) {
        Ok(mut e) => {
            for d in &proj.devices {
                for b in e.boot_errors(d.id) {
                    lines.push(format!("Fehler: {}, startup-config Zeile {}: {}\n    {}\n", d.name, b.line, b.text, b.msg));
                    errors += 1;
                }
            }
            // the faults its lab starts with: on this engine, as when it loads
            let (acts, bad) = jfn_sim::runner::start_actions(proj, &e);
            for why in bad {
                lines.push(format!("Fehler: Startfehler {}\n", why));
                errors += 1;
            }
            for a in acts {
                if let Err(x) = e.apply(a) {
                    lines.push(format!("Fehler: Startfehler: {}\n", x.msg().text()));
                    errors += 1;
                }
            }
        }
        Err(e) => {
            lines.push(format!("Fehler: das Projekt läuft nicht: {}\n", e));
            errors += 1;
        }
    }
    (lines, errors)
}

/// what is wrong with the course in folder `dir` (without a trailing '/'): its lines, how many of
/// them keep it from being offered.  The labs get what `check` finds as well.
pub fn check_course(dir: &str) -> (Vec<String>, usize) {
    let (found, labs) = course_findings(dir, base_name(dir));
    let mut errors = found.iter().filter(|p| p.fatal).count();
    let mut lines: Vec<String> = found.iter().map(|p| format!("{}{}\n", if p.fatal { "Fehler: " } else { "Hinweis: " }, p.msg.text())).collect();
    for (file, more, e) in labs {
        lines.extend(more.into_iter().map(|l| format!("{}: {}", file, l)));
        errors += e;
    }
    (lines, errors)
}

/// what is wrong with the course `id` in folder `dir`, whatever that folder is called (the installer checks it
/// before it gets its name): the findings of the course, then per lab what `check` finds (its file, its
/// lines, how many of them are errors)
pub fn course_findings(dir: &str, id: &str) -> (Vec<jfn_model::academy::Problem>, Vec<(String, Vec<String>, usize)>) {
    use jfn_model::academy::{fault_issues, lab_issues, load, quiz_issues, safe_path, StepKind, COURSE_FILE, LANGS};
    let read = |rel: &str| plat::read_file(&format!("{}/{}", dir, rel)).map(|b| String::from_utf8_lossy(&b).into_owned());
    let exists = |rel: &str| plat::file_exists(&format!("{}/{}", dir, rel)) == 1;
    let (course, mut found) = load(id, read(COURSE_FILE).as_deref(), &exists);
    let mut labs = Vec::new();
    // a path that was refused is not read
    for s in course.iter().flat_map(|c| c.steps()).filter(|s| safe_path(&s.file).is_ok()) {
        match s.kind() {
            Some(StepKind::Lesson) => {
                for lang in LANGS {
                    if let (Some(md), Some(c)) = (read(&format!("{}/{}", lang, s.file)), &course) {
                        found.extend(c.lesson_issues(&s.id, &md, &exists));
                    }
                }
            }
            Some(k @ (StepKind::Lab | StepKind::Fault)) => {
                if let Some(t) = read(&s.file) {
                    found.extend(if k == StepKind::Lab { lab_issues(&s.id, &t) } else { fault_issues(&s.id, &t) });
                    if let Ok(p) = Project::from_text(&t) {
                        labs.push((s.file.clone(), p));
                    }
                }
            }
            Some(k @ (StepKind::Quiz | StepKind::Exam)) => {
                if let Some(t) = read(&s.file) {
                    found.extend(quiz_issues(&s.id, k, &t));
                }
            }
            None => {}
        }
    }
    let checked = labs
        .into_iter()
        .map(|(file, p)| {
            let (lines, errors) = check_project(&p);
            (file, lines, errors)
        })
        .collect();
    (found, checked)
}

/// jfn-sim course DIR: the lines of check_course and a last line; 0: nothing keeps it from being offered
pub fn course(dir: &str) -> i32 {
    let dir = dir.trim_end_matches(['/', '\\']);
    let (lines, errors) = check_course(dir);
    for l in &lines {
        say(l);
    }
    say(&format!("{}: {}\n", base_name(dir), if errors == 0 { String::from("in Ordnung") } else { format!("{} Fehler", errors) }));
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

const USAGE: &str = "jfn-sim -- JFNetworker ohne Fenster\n\n  jfn-sim run PROJEKT [--scenario S] [--pcap F] [--report F] [--golden F] [--series F]\n  jfn-sim check PROJEKT\n  jfn-sim course ORDNER\n  jfn-sim course-install QUELLE KURS|all --into ORDNER [--index DATEI] [--stop-after BYTES]\n  jfn-sim course-list QUELLE [--into ORDNER] [--index DATEI]\n  jfn-sim course-remove KURS --into ORDNER\n  jfn-sim decode HEX\n  jfn-sim gen N [--out DATEI]\n  jfn-sim rewind PROJEKT [SEKUNDEN] [--traffic K]\n  jfn-sim sweep DATEI [PROJEKT] [--threads N] [--csv F] [--detail F] [--report F] [--golden F] [--name N] [--timing]\n";

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
        Some("course") => match args.get(2) {
            Some(d) => course(d),
            None => {
                complain(USAGE);
                2
            }
        },
        Some("course-install") => install::cli_install(&args[2..]),
        Some("course-list") => install::cli_list(&args[2..]),
        Some("course-remove") => install::cli_remove(&args[2..]),
        Some("decode") if args.len() > 2 => decode(&args[2..].join(" ")),
        Some("gen") => generate(&args[2..]),
        Some("rewind") => rewind(&args[2..]),
        Some("sweep") => sweep(&args[2..]),
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

// ---------------------------------------------------------------- parameter sweeps (SA2)

/// a worker's stack
const SWEEP_STACK: usize = 4 << 20;

/// a warm engine with what is left of its levels: Clone needs no lock, but an Engine is Send only
type WarmEngine = Arc<Mutex<Engine>>;

/// what the workers take from
struct SweepQueue {
    /// the next boot (config * seeds + seed)
    next_boot: usize,
    /// levels ready to run: (warm engine, config, seed, level)
    levels: VecDeque<(WarmEngine, usize, usize, usize)>,
    /// boots under way: their levels will come
    booting: usize,
}

struct SweepWork {
    plan: Plan,
    queue: Mutex<SweepQueue>,
    figs: Mutex<Vec<Option<Fig>>>,
    ctl: Arc<SweepCtl>,
}

/// what a sweep under way tells and takes: how many runs are done, cancel
#[derive(Debug, Default)]
pub struct SweepCtl {
    done: AtomicUsize,
    cancel: Arc<AtomicBool>,
}

impl SweepCtl {
    pub fn new() -> SweepCtl {
        SweepCtl::default()
    }
    /// the runs finished so far
    pub fn done(&self) -> usize {
        self.done.load(Ordering::Relaxed)
    }
    /// the workers stop at the next whole second of virtual time
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
}

/// takes work until there is none: levels before boots, which bounds the warm engines held
fn sweep_worker(w: &SweepWork) {
    let (seeds, levels) = (w.plan.seeds.len(), w.plan.levels.len());
    let boots = w.plan.cfgs.len() * seeds;
    loop {
        if w.ctl.cancelled() {
            return;
        }
        let (level, boot) = {
            let mut q = w.queue.lock();
            match q.levels.pop_front() {
                Some(l) => (Some(l), None),
                None if q.next_boot < boots => {
                    q.next_boot += 1;
                    q.booting += 1;
                    (None, Some(q.next_boot - 1))
                }
                None if q.booting > 0 => (None, None),
                None => return,
            }
        };
        match (level, boot) {
            (Some((warm, c, s, l)), _) => {
                let mut e = warm.lock().clone();
                drop(warm);
                let f = w.plan.run_level(&mut e, l);
                drop(e);
                w.figs.lock()[w.plan.index(c, s, l)] = Some(f);
                w.ctl.done.fetch_add(1, Ordering::Relaxed);
            }
            (None, Some(b)) => {
                let (c, s) = (b / seeds, b % seeds);
                match w.plan.warm(c, s) {
                    Ok(e) => {
                        let warm: WarmEngine = Arc::new(Mutex::new(e));
                        let mut q = w.queue.lock();
                        for l in 0..levels {
                            q.levels.push_back((warm.clone(), c, s, l));
                        }
                        q.booting -= 1;
                    }
                    Err(_) => {
                        let mut figs = w.figs.lock();
                        for l in 0..levels {
                            figs[w.plan.index(c, s, l)] = Some(Fig { status: Status::Failed, ..Fig::default() });
                        }
                        drop(figs);
                        w.ctl.done.fetch_add(levels, Ordering::Relaxed);
                        w.queue.lock().booting -= 1;
                    }
                }
            }
            // the others boot: their levels come
            _ => plat::thread_yield(),
        }
    }
}

/// the sweep on `threads` threads (this one among them; where no thread starts, on those that did)
pub fn run_sweep(plan: &Plan, threads: usize) -> Sweep {
    plan.collect(run_figs(plan, threads, &Arc::new(SweepCtl::new())))
}

/// run_sweep that counts its runs and can be cancelled: None when it was
pub fn run_sweep_ctl(plan: &Plan, threads: usize, ctl: &Arc<SweepCtl>) -> Option<Sweep> {
    let mut p = plan.clone();
    p.stop = Stop(Some(ctl.cancel.clone()));
    let figs = run_figs(&p, threads, ctl);
    if ctl.cancelled() {
        return None;
    }
    Some(plan.collect(figs))
}

/// the runs on `threads` threads, each result at its place
fn run_figs(plan: &Plan, threads: usize, ctl: &Arc<SweepCtl>) -> Vec<Option<Fig>> {
    let work = Arc::new(SweepWork { plan: plan.clone(), queue: Mutex::new(SweepQueue { next_boot: 0, levels: VecDeque::new(), booting: 0 }), figs: Mutex::new(alloc::vec![None; plan.runs()]), ctl: ctl.clone() });
    let mut handles = Vec::new();
    for _ in 1..threads.max(1) {
        let w = work.clone();
        if let Some(h) = plat::thread_spawn(SWEEP_STACK, Box::new(move || sweep_worker(&w))) {
            handles.push(h);
        }
    }
    sweep_worker(&work);
    for h in handles {
        h.join();
    }
    core::mem::take(&mut *work.figs.lock())
}

/// how a job ended
#[derive(Debug)]
pub enum SweepOut {
    Done(Sweep),
    /// the experiment does not fit the project (Plan::new)
    Refused(Vec<Msg>),
    /// cancelled: nothing comes back
    Cancelled,
}

/// jobs under way, of every window (ui.sync_hook_set takes a plain fn)
static JOBS: AtomicU32 = AtomicU32::new(0);

/// a job is still under way (the sync hook of test scripts)
pub fn sweeps_pending() -> bool {
    JOBS.load(Ordering::Acquire) > 0
}

/// a sweep on threads of its own; dropping it cancels it and waits for its threads
pub struct SweepJob {
    ctl: Arc<SweepCtl>,
    total: usize,
    out: Arc<Mutex<Option<SweepOut>>>,
    thread: Option<plat::Thread>,
}

impl SweepJob {
    /// the job runs from these copies, so nothing that changes afterwards touches it; None when no
    /// thread can be started
    pub fn start(exp: Experiment, project: Project, name: String, threads: usize) -> Option<SweepJob> {
        let levels = if exp.load.rps.is_empty() { exp.load.users.len() } else { exp.load.rps.len() };
        let total = exp.configs().len() * exp.seeds_or(project.seed).len() * levels;
        let ctl = Arc::new(SweepCtl::new());
        let out: Arc<Mutex<Option<SweepOut>>> = Arc::new(Mutex::new(None));
        let (c, o) = (ctl.clone(), out.clone());
        JOBS.fetch_add(1, Ordering::AcqRel);
        let thread = plat::thread_spawn(
            SWEEP_STACK,
            Box::new(move || {
                let res = match Plan::new(&exp, &project, &name) {
                    Err(found) => SweepOut::Refused(found),
                    Ok(_) if c.cancelled() => SweepOut::Cancelled,
                    Ok(plan) => match run_sweep_ctl(&plan, threads, &c) {
                        Some(s) => SweepOut::Done(s),
                        None => SweepOut::Cancelled,
                    },
                };
                *o.lock() = Some(res);
                JOBS.fetch_sub(1, Ordering::AcqRel);
            }),
        );
        if thread.is_none() {
            JOBS.fetch_sub(1, Ordering::AcqRel);
            return None;
        }
        Some(SweepJob { ctl, total, out, thread })
    }
    /// the runs finished so far
    pub fn done(&self) -> usize {
        self.ctl.done()
    }
    /// the runs of the whole job
    pub fn total(&self) -> usize {
        self.total
    }
    /// the job ends at the next whole second of virtual time, without a result
    pub fn cancel(&self) {
        self.ctl.cancel();
    }
    pub fn cancelled(&self) -> bool {
        self.ctl.cancelled()
    }
    /// how it ended, once it has
    pub fn take(&mut self) -> Option<SweepOut> {
        let out = self.out.lock().take()?;
        if let Some(t) = self.thread.take() {
            t.join();
        }
        Some(out)
    }
}

impl Drop for SweepJob {
    fn drop(&mut self) {
        self.ctl.cancel();
        if let Some(t) = self.thread.take() {
            t.join();
        }
    }
}

/// what a sweep is given
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct SweepOpts {
    file: Option<String>,
    project: Option<String>,
    name: Option<String>,
    threads: Option<usize>,
    csv: Option<String>,
    detail: Option<String>,
    report: Option<String>,
    golden: Option<String>,
    timing: bool,
    /// with the recording on: only to see what the quiet trace saves
    loud: bool,
}

fn sweep_opts(args: &[String]) -> Result<SweepOpts, String> {
    let mut o = SweepOpts::default();
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
            "--sweep" => o.file = Some(value()?),
            "--threads" => {
                let v = value()?;
                o.threads = Some(v.parse::<usize>().ok().filter(|&n| n > 0).ok_or_else(|| format!("--threads braucht eine Zahl, nicht {}.", v))?);
            }
            "--csv" => o.csv = Some(value()?),
            "--detail" => o.detail = Some(value()?),
            "--report" => o.report = Some(value()?),
            "--golden" => o.golden = Some(value()?),
            "--name" => o.name = Some(value()?),
            "--timing" => o.timing = true,
            "--loud" => o.loud = true,
            "--script" | "--shot" | "--style" | "--lang" | "--academy-dir" => {
                value()?;
            }
            k if k.starts_with('-') => {}
            p => {
                if o.file.is_none() && p.ends_with(".jfnex") {
                    o.file = Some(String::from(p));
                } else if o.project.is_none() {
                    o.project = Some(String::from(p));
                } else {
                    return Err(format!("Was soll {} sein? Es gibt schon {}.", p, o.project.as_deref().unwrap_or("")));
                }
            }
        }
        i += 1;
    }
    // a project alone: its own section
    if o.file.is_none() {
        o.file = o.project.take();
    }
    Ok(o)
}

/// the experiment and its project: a .jfnex file names its project, relative to itself (a project on
/// the command line goes first); a project file holds them in its section (--name picks one)
fn sweep_input(o: &SweepOpts) -> Result<(Experiment, Project, String), String> {
    let Some(file) = o.file.as_deref() else {
        return Err(String::from("Keine Experiment-Datei angegeben."));
    };
    if file.ends_with(".jfnex") {
        let x = Experiment::from_text(&read_text(file)?).map_err(|e| format!("{}: {}", file, e))?;
        let path = match o.project.as_deref() {
            Some(p) => String::from(p),
            None if x.project.is_empty() => return Err(format!("{}: es nennt kein Projekt (\u{201e}project\u{201c}).", file)),
            None => {
                let base = base_name(file);
                let dir = &file[..file.len() - base.len()];
                format!("{}{}", dir, x.project)
            }
        };
        // (what is not named on the command line stays closed to a program with PERMISSIONS)
        let p = load(&path).map_err(|e| format!("{}\nDas Projekt gehört auf die Befehlszeile: jfn-sim sweep {} {}", e, file, path))?;
        return Ok((x, p, String::from(base_name(file))));
    }
    let p = load(file)?;
    let x = match o.name.as_deref() {
        Some(n) => p.experiments.iter().find(|x| x.name.eq_ignore_ascii_case(n)).ok_or_else(|| format!("{}: ein Experiment \u{201e}{}\u{201c} gibt es nicht.", file, n))?,
        None => match &p.experiments[..] {
            [x] => x,
            [] => return Err(format!("{}: das Projekt enthält kein Experiment.", file)),
            _ => return Err(format!("{}: das Projekt enthält mehrere Experimente; --name wählt eines.", file)),
        },
    };
    let x = x.clone();
    Ok((x, p, String::from(base_name(file))))
}

/// jfn-sim sweep, the GUI program's --sweep
pub fn sweep(args: &[String]) -> i32 {
    let o = match sweep_opts(args) {
        Ok(o) => o,
        Err(e) => {
            complain(&format!("{}\n", e));
            return 2;
        }
    };
    let (exp, proj, name) = match sweep_input(&o) {
        Ok(x) => x,
        Err(e) => {
            complain(&format!("{}\n", e));
            return 2;
        }
    };
    let mut plan = match Plan::new(&exp, &proj, &name) {
        Ok(p) => p,
        Err(found) => {
            for m in found {
                complain(&format!("{}: {}\n", name, m));
            }
            return 2;
        }
    };
    plan.quiet = !o.loud;
    let threads = o.threads.unwrap_or_else(|| plat::cpu_count().max(1) as usize);
    let t0 = plat::time_us();
    let out = run_sweep(&plan, threads);
    let wall = plat::time_us() - t0;
    let table = out.table();
    say(&table);
    if o.timing {
        let events: u64 = out.details.iter().map(|f| f.events).sum();
        let answers: u64 = out.details.iter().map(|f| f.responses).sum();
        let ms = wall.max(1) / 1000;
        complain(&format!("{} Läufe, {} Threads: {} ms, {} Ereignisse gemessen ({} je Sekunde), {} Antworten gemessen ({} je Sekunde)\n", plan.runs(), threads, ms, events, events * 1_000_000 / wall.max(1), answers, answers * 1_000_000 / wall.max(1)));
    }
    let mut code = 0;
    for (f, text) in [(o.report.as_deref(), Some(&table)), (o.csv.as_deref(), None), (o.detail.as_deref(), None)] {
        let Some(f) = f else { continue };
        let body = match text {
            Some(t) => t.clone(),
            None if o.csv.as_deref() == Some(f) => out.csv(),
            None => out.detail_csv(),
        };
        if !plat::write_file(f, body.as_bytes()) {
            complain(&format!("{} lässt sich nicht schreiben.\n", f));
            code = 2;
        }
    }
    if let Some(g) = o.golden.as_deref() {
        match read_text(g) {
            Ok(want) => {
                let want = want.replace("\r\n", "\n");
                if want != table {
                    let (wl, gl): (Vec<&str>, Vec<&str>) = (want.lines().collect(), table.lines().collect());
                    let i = wl.iter().zip(gl.iter()).position(|(a, b)| a != b).unwrap_or(wl.len().min(gl.len()));
                    complain(&format!("FAIL sweep: die Tabelle weicht von {} ab (Zeile {}):\n- {}\n+ {}\n", g, i + 1, wl.get(i).unwrap_or(&""), gl.get(i).unwrap_or(&"")));
                    code = code.max(1);
                }
            }
            Err(e) => {
                complain(&format!("{}\n", e));
                code = 2;
            }
        }
    }
    code
}
