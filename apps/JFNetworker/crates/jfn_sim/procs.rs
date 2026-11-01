// procs -- the processes of a device as the shell's ps sees them, and the CPU time its services
// used second by second (the Systemmonitor of a host's desktop; ps formats the same rows)
//
//   Engine::processes(dev) -> Vec<ProcRow>    init (1), the shell of every command-line session
//                                             (the console's 100, cli/ses.rs), every running
//                                             service (PID 100 + its id; the command line of the
//                                             background job that runs it, if one does) and the
//                                             background jobs that run no service, by PID; with
//                                             their terminal (ps's TTY: "?" for a service no
//                                             session runs), their CPU time (scripts, jfs
//                                             processes and web servers count theirs, the rest 0)
//                                             and whether the console's kill ends them (the
//                                             console's background jobs)
//   Engine::cpu_secs(dev, secs) -> Vec<u64>   the CPU time (ns) the device's programs used in
//                                             each of the last `secs` seconds up to the second of
//                                             now, oldest first: the busy time of its scripts,
//                                             jfs processes and web servers (top's %CPU)
//
// Both only read the engine: no state of their own, nothing for the keyframes, no notes.

use alloc::string::String;
use alloc::vec::Vec;

use crate::cli::sh::SHELL_PID;
use crate::cli::shrun::JfsProc;
use crate::httpsvc::HttpServer;
use crate::kernel::{Engine, SEC};
use crate::scriptsvc::ScriptService;

/// what a process is
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProcKind {
    /// PID 1
    Init,
    /// the login shell of a command-line session (the console's: PID 100)
    Shell,
    /// a service of the device, or a program a background job runs as one
    Service,
    /// a background job of the shell that runs no service (sleep, ping ...)
    Job,
}

/// one row of ps
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcRow {
    pub pid: u32,
    pub user: String,
    /// its terminal as ps names it ("tty1", "pts/0"; "?" none)
    pub tty: String,
    /// CPU time in ns
    pub cpu_ns: u64,
    /// the command: a service's name, a background job's command line
    pub cmd: String,
    pub kind: ProcKind,
    /// the service it is (Service), else None
    pub svc: Option<String>,
    /// the console's kill ends it (a background job of the console's shell, also one that runs
    /// a program)
    pub killable: bool,
}

impl Engine {
    /// the processes of a device, by PID (no shell: init and the services)
    pub fn processes(&self, dev: u32) -> Vec<ProcRow> {
        let row = |pid: u32, tty: &str, cpu_ns: u64, cmd: String, kind: ProcKind, svc: Option<String>, killable: bool| ProcRow { pid, user: String::from("root"), tty: String::from(tty), cpu_ns, cmd, kind, svc, killable };
        let mut rows: Vec<ProcRow> = alloc::vec![row(1, "?", 0, String::from("init"), ProcKind::Init, None, false)];
        let procs = self.node_index(dev).map(|ni| self.ses_procs(ni)).unwrap_or_default();
        if procs.is_empty() {
            rows.push(row(SHELL_PID, "tty1", 0, String::from("-sh"), ProcKind::Shell, None, false));
        }
        for (_, tty, pid, _, _) in &procs {
            rows.push(row(if *pid == 0 { SHELL_PID } else { *pid }, tty, 0, String::from("-sh"), ProcKind::Shell, None, false));
        }
        let Some(ni) = self.node_index(dev) else { return rows };
        // the background jobs that still run, of every session: PID, the service they run, their
        // command line, their terminal, whether they are the console's
        let jobs: Vec<(u32, Option<u32>, String, String, bool)> = procs.iter().flat_map(|(id, tty, _, _, js)| js.iter().map(move |(p, s, t)| (*p, *s, t.clone(), tty.clone(), *id == 0))).collect();
        // the programs in the foreground of the sessions: service, terminal
        let fg: Vec<(u32, String)> = procs.iter().filter_map(|(_, tty, _, f, _)| f.map(|s| (s, tty.clone()))).collect();
        let mut shown: Vec<u32> = Vec::new();
        if let Some(os) = self.os(ni) {
            for s in os.svcs.iter().filter(|s| s.running) {
                let pid = 100 + s.id;
                let job = jobs.iter().find(|j| j.1 == Some(s.id));
                let (name, tty) = match job {
                    Some(j) => {
                        shown.push(j.0);
                        (j.2.clone(), j.3.clone())
                    }
                    None => (s.name.clone(), fg.iter().find(|f| f.0 == s.id).map_or_else(|| String::from("?"), |f| f.1.clone())),
                };
                let cpu = s.svc.as_ref().map_or(0, |x| {
                    if let Some(sc) = x.as_any().downcast_ref::<ScriptService>() {
                        sc.busy_ns()
                    } else if let Some(p) = x.as_any().downcast_ref::<JfsProc>() {
                        p.inner.busy_ns()
                    } else if let Some(h) = x.as_any().downcast_ref::<HttpServer>() {
                        h.busy_ns
                    } else {
                        0
                    }
                });
                rows.push(row(pid, &tty, cpu, name, ProcKind::Service, Some(s.name.clone()), job.is_some_and(|j| j.4)));
            }
        }
        for (pid, svc, text, tty, con) in &jobs {
            if !shown.contains(pid) && svc.is_none_or(|s| !rows.iter().any(|r| r.pid == 100 + s)) {
                rows.push(row(*pid, tty, 0, text.clone(), ProcKind::Job, None, *con));
            }
        }
        rows.sort_by_key(|r| r.pid);
        rows
    }

    /// the CPU time (ns) of the device's programs in each of the last `secs` seconds, oldest
    /// first (the newest is the second of now)
    pub fn cpu_secs(&self, dev: u32, secs: u64) -> Vec<u64> {
        let mut out = alloc::vec![0u64; secs as usize];
        let Some(os) = self.node_index(dev).and_then(|ni| self.os(ni)) else { return out };
        let now = self.now;
        for s in os.svcs.iter().filter(|s| s.running) {
            let Some(x) = s.svc.as_ref() else { continue };
            let any = x.as_any();
            for (i, v) in out.iter_mut().enumerate() {
                let back = secs - 1 - i as u64;
                let Some(t) = now.checked_sub(back * SEC) else { continue };
                *v += if let Some(sc) = any.downcast_ref::<ScriptService>() {
                    sc.load(t, 1)
                } else if let Some(p) = any.downcast_ref::<JfsProc>() {
                    p.inner.load(t, 1)
                } else if let Some(h) = any.downcast_ref::<HttpServer>() {
                    h.load(t, 1).0
                } else {
                    0
                };
            }
        }
        out
    }
}
