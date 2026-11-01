// desktoptest -- checks of what a host's desktop builds on (group desk of --selftest): the
// processes of jfn_sim/procs.rs against ps (a background job, its PID and command, killable, a
// service that is not, the job gone after kill), the CPU time per second against the web
// server's own numbers, the Dateimanager's names, copies and moves on a store (devwin/
// filemgr.rs), the editor's paths, which window draws the console (devwin/desktop.rs), and the
// labels' contrast on every background of the desktop in both themes
//
// The project is tests/fixtures/shell_tour.jfnet: PC1 (10.0.0.1), SRV1 (10.0.0.10, a web
// server "web"), SW1, R1.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use jfn_model::files::Store;
use jfn_model::project::Project;
use jfn_sim::{Engine, ProcKind, Report, MS, SEC};
use ui::{DeskState, DW_MIN};

use crate::selftest::T;
use crate::workspace::devwin::desktop::{self, Desk, Prog};
use crate::workspace::devwin::{editor, filemgr};

const FIX: &str = include_str!("../tests/fixtures/shell_tour.jfnet");
const BUDGET: usize = 4_000_000;
const PC1: u32 = 1;
const SRV1: u32 = 2;

fn s(x: &str) -> String {
    String::from(x)
}

fn engine() -> Engine {
    let p = Project::from_text(FIX).unwrap_or_else(|_| Project::new("desk", 13));
    Engine::from_project(&p).unwrap_or_else(|_| Engine::new(0))
}

/// lines typed at a device, then `d` of virtual time: what it printed
fn cli(e: &mut Engine, dev: u32, line: &str, d: u64) -> String {
    let _ = e.take_reports();
    let _ = e.cli_line(dev, line);
    let t = e.now() + d;
    e.run_until(t, BUDGET);
    let mut o = String::new();
    for r in e.take_reports() {
        if let Report::CliOut { dev: x, bytes, .. } = r
            && x == dev
        {
            o.push_str(&String::from_utf8_lossy(&bytes));
        }
    }
    o
}

pub(crate) fn run(t: &mut T) {
    procs_t(t);
    cpu_t(t);
    files_t(t);
    console_t(t);
    contrast_t(t);
}

// the processes as ps shows them, a background job ended with kill
fn procs_t(t: &mut T) {
    let mut e = engine();
    let base = e.processes(PC1);
    t.eq("init and the shell", base.iter().map(|p| (p.pid, p.cmd.clone(), p.kind)).take(2).collect::<Vec<_>>(), alloc::vec![(1, s("init"), ProcKind::Init), (100, s("-sh"), ProcKind::Shell)]);
    let started = cli(&mut e, PC1, "sleep 100 &", 10 * MS);
    let rows = e.processes(PC1);
    let job = rows.iter().find(|p| p.cmd == "sleep 100").cloned();
    t.check("the job is a process", job.as_ref().is_some_and(|j| j.kind == ProcKind::Job && j.killable && j.user == "root"));
    let pid = job.as_ref().map_or(0, |j| j.pid);
    t.eq("its PID as the shell printed it", started, format!("[1] {}\n", pid));
    // ps lists the same rows (and itself)
    let ps = cli(&mut e, PC1, "ps", 0);
    let listed: Vec<(u32, String)> = ps.lines().skip(1).filter_map(|l| {
        let w: Vec<&str> = l.split_whitespace().collect();
        Some((w.first()?.parse().ok()?, w.get(4..)?.join(" ")))
    }).collect();
    let mut want: Vec<(u32, String)> = rows.iter().map(|p| (p.pid, p.cmd.clone())).collect();
    want.push((want.iter().map(|w| w.0).max().unwrap_or(100) + 1, s("ps")));
    t.eq("ps shows the rows", listed, want);
    // kill ends it: gone from the processes
    let _ = cli(&mut e, PC1, &format!("kill {}", pid), 10 * MS);
    t.check("killed: gone", !e.processes(PC1).iter().any(|p| p.pid == pid));
    // SRV1's web server is a service, no job the shell may end
    let web = e.processes(SRV1).into_iter().find(|p| p.svc.as_deref() == Some("web"));
    t.check("a service is no job", web.as_ref().is_some_and(|w| w.kind == ProcKind::Service && !w.killable && w.pid > 100));
    t.eq("an unknown device", e.processes(99).len(), 2);
}

// the CPU time per second: what the web server counts for itself
fn cpu_t(t: &mut T) {
    let mut e = engine();
    t.eq("a minute of seconds", e.cpu_secs(SRV1, 60).len(), 60);
    t.check("nothing at first", e.cpu_secs(SRV1, 60).iter().all(|&x| x == 0));
    for _ in 0..3 {
        let _ = cli(&mut e, PC1, "curl -s http://10.0.0.10/ > /dev/null", 2 * SEC);
    }
    let secs = e.cpu_secs(SRV1, 60);
    let own = e.service::<jfn_sim::HttpServer>(SRV1, "web").map_or(0, |h| h.load(e.now(), 60).0);
    t.check("the web server worked", own > 0);
    t.eq("the seconds add up to its load", secs.iter().sum::<u64>(), own);
    t.eq("PC1 used none", e.cpu_secs(PC1, 60).iter().sum::<u64>(), 0);
    t.eq("as a share of the CPUs", crate::workspace::devwin::sysmon::cpu_percent(SEC), 250);
}

// the Dateimanager's model: free names, copies of a folder tree, moves; the editor's paths
fn files_t(t: &mut T) {
    let mut st = Store::new(false, 1 << 30, None);
    let _ = st.mkdir("/root/app", 5, false);
    let _ = st.write("/root/app/a.txt", b"hallo\n", 5);
    let _ = st.mkdir("/root/app/lib", 5, false);
    let _ = st.write("/root/app/lib/m.jfs", b"let x = 1\n", 5);
    let _ = st.write("/root/notes.txt", b"n\n", 5);
    t.eq("a free name", filemgr::paste_name(&st, "/root", "neu"), s("neu"));
    t.eq("a taken file", filemgr::paste_name(&st, "/root", "notes.txt"), s("notes (Kopie).txt"));
    t.eq("a taken folder", filemgr::paste_name(&st, "/root", "app"), s("app (Kopie)"));
    // a folder with all in it into its own parent: a copy beside it
    let c = filemgr::copy_into(&mut st, "/root/app", "/root", 9);
    t.eq("copied folder", c, Ok(s("/root/app (Kopie)")));
    t.eq("its file", st.read("/root/app (Kopie)/lib/m.jfs").map(Vec::from), Ok(Vec::from(&b"let x = 1\n"[..])));
    t.eq("the copy's time", st.stat("/root/app (Kopie)/a.txt").map(|x| x.mtime), Some(9));
    t.eq("a second copy", filemgr::copy_into(&mut st, "/root/app", "/root", 9), Ok(s("/root/app (Kopie 2)")));
    t.check("not into itself", filemgr::copy_into(&mut st, "/root/app", "/root/app/lib", 9).is_err());
    // moving keeps the times; within its own folder nothing happens
    t.eq("moved", filemgr::move_into(&mut st, "/root/notes.txt", "/root/app"), Ok(s("/root/app/notes.txt")));
    t.eq("its time stays", st.stat("/root/app/notes.txt").map(|x| x.mtime), Some(5));
    t.check("the old place is empty", !st.exists("/root/notes.txt"));
    t.eq("in place", filemgr::move_into(&mut st, "/root/app/a.txt", "/root/app"), Ok(s("/root/app/a.txt")));
    t.eq("join", (editor::join("/", "etc"), editor::join("/root", "a.txt"), editor::join("/root", "/tmp/x")), (s("/etc"), s("/root/a.txt"), s("/tmp/x")));
}

// the window that draws the console: Terminal or MIB-Browser, the higher one not minimised
fn console_t(t: &mut T) {
    let mut d = Desk::new(PC1);
    t.eq("no window", d.console_owner(), None);
    d.open_prog(Prog::Editor);
    t.eq("no console window", d.console_owner(), None);
    d.open_prog(Prog::Terminal);
    d.open_prog(Prog::Mib);
    t.eq("new windows: the later one", d.console_owner(), Some(Prog::Mib));
    // the state knows them: Terminal (1) above the MIB-Browser (7)
    d.desk = DeskState::load("desk 1\nwin 7 10 10 300 200 0\nwin 1 20 20 300 200 0\nactive 1\n").unwrap_or_default();
    t.eq("the higher one", d.console_owner(), Some(Prog::Terminal));
    d.desk.minimize(Prog::Terminal.key());
    t.check("minimised", d.desk.state(Prog::Terminal.key()) & DW_MIN != 0);
    t.eq("the other one while it is minimised", d.console_owner(), Some(Prog::Mib));
    d.desk.minimize(Prog::Mib.key());
    t.eq("both minimised", d.console_owner(), None);
    t.eq("keys", (Prog::Terminal.key(), Prog::Ide.key(), Prog::of_key(7), Prog::of_key(0), Prog::of_key(11)), (1, 10, Some(Prog::Mib), None, None));
}

// every background in both themes: labels 4.5:1 against it and against its pattern
fn contrast_t(t: &mut T) {
    for bg in 0..desktop::BG_LABEL {
        for dark in [false, true] {
            let (c, fg, pat) = desktop::bg_colors(bg, dark);
            let worst = [c, pat[0], pat[1]].iter().map(|&x| ui::contrast_ratio(fg, x)).fold(f32::MAX, f32::min);
            t.check(&format!("labels on background {} ({}): {:.2}", bg, if dark { "dark" } else { "light" }, worst), worst >= 4.5);
        }
    }
}
