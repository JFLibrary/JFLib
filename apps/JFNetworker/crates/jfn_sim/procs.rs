// procs -- the processes of a device as the shell's ps sees them, and the CPU time its services
// used second by second (the Systemmonitor of a host's desktop; ps formats the same rows)
//
//   Engine::processes(dev) -> Vec<ProcRow>    init (1), the shell (100), every running service
//                                             (PID 100 + its id; the command line of the
//                                             background job that runs it, if one does) and the
//                                             background jobs that run no service, by PID; with
//                                             their CPU time (scripts, jfs processes and web
//                                             servers count theirs, the rest 0) and whether the
//                                             shell's kill ends them (the background jobs)
//   Engine::cpu_secs(dev, secs) -> Vec<u64>   the CPU time (ns) the device's programs used in
//                                             each of the last `secs` seconds up to the second of
//                                             now, oldest first: the busy time of its scripts,
//                                             jfs processes and web servers (top's %CPU)
//
// Both only read the engine: no state of their own, nothing for the keyframes, no notes.

use alloc::string::String;
use alloc::vec::Vec;

use crate::cli::print::JobOut;
use crate::cli::sh::SHELL_PID;
use crate::cli::shrun::JfsProc;
use crate::cli::CliJob;
use crate::httpsvc::HttpServer;
use crate::kernel::{Engine, SEC};
use crate::scriptsvc::ScriptService;

/// what a process is
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProcKind {
    /// PID 1
    Init,
    /// the login shell of the console (PID 100)
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
    /// CPU time in ns
    pub cpu_ns: u64,
    /// the command: a service's name, a background job's command line
    pub cmd: String,
    pub kind: ProcKind,
    /// the service it is (Service), else None
    pub svc: Option<String>,
    /// the shell's kill ends it (a background job, also one that runs a program)
    pub killable: bool,
}

impl Engine {
    /// the processes of a device, by PID (no shell: init and the services)
    pub fn processes(&self, dev: u32) -> Vec<ProcRow> {
        let row = |pid: u32, cpu_ns: u64, cmd: String, kind: ProcKind, svc: Option<String>, killable: bool| ProcRow { pid, user: String::from("root"), cpu_ns, cmd, kind, svc, killable };
        let mut rows: Vec<ProcRow> = alloc::vec![row(1, 0, String::from("init"), ProcKind::Init, None, false), row(SHELL_PID, 0, String::from("-sh"), ProcKind::Shell, None, false)];
        let Some(ni) = self.node_index(dev) else { return rows };
        // the background jobs that still run: PID, the service they run, their command line
        let jobs: Vec<(u32, Option<u32>, String)> = self
            .sh(ni)
            .map(|s| {
                s.bg.iter()
                    .filter(|j| j.ended.is_none())
                    .map(|j| {
                        let svc = match j.cj {
                            Some(CliJob { out: JobOut::Proc { svc, .. }, .. }) => Some(svc),
                            _ => None,
                        };
                        (j.pid, svc, j.text.clone())
                    })
                    .collect()
            })
            .unwrap_or_default();
        let mut shown: Vec<u32> = Vec::new();
        if let Some(os) = self.os(ni) {
            for s in os.svcs.iter().filter(|s| s.running) {
                let pid = 100 + s.id;
                let job = jobs.iter().find(|j| j.1 == Some(s.id));
                let name = match job {
                    Some(j) => {
                        shown.push(j.0);
                        j.2.clone()
                    }
                    None => s.name.clone(),
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
                rows.push(row(pid, cpu, name, ProcKind::Service, Some(s.name.clone()), job.is_some()));
            }
        }
        for (pid, svc, text) in &jobs {
            if !shown.contains(pid) && svc.is_none_or(|s| !rows.iter().any(|r| r.pid == 100 + s)) {
                rows.push(row(*pid, 0, text.clone(), ProcKind::Job, None, true));
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
