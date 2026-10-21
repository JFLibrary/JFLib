// cdp -- continuous data protection: a job with cdp watches its sources and gets a restore
// point soon after changes, at most every cdp_rpo seconds
//
//   start(ctx)        the manager (a thread of main): one watcher thread per CDP job; it
//                     follows jobs that are added, changed, disabled or removed
//   status(ctx)       GET /api/cdp
//
// Sources on the server: plat::watch.  Sources on an agent: GET /v1/changes there (a long
// poll).  A run passes the changed paths to the backup (engine::Hint): folders without changes
// are taken over from the previous point without listing them.  When events were lost
// (overflow) the next run walks everything.  Changes below excluded names do not start runs.

use alloc::boxed::Box;
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, Ordering};

use bc_proto::model::*;
use netio::http::Response;
use plat::sync::Mutex;

use crate::ctx::{Ctx, C_HOSTS, C_JOBS, C_POINTS};
use crate::util::{all, list, now, rec};

struct Worker {
    stop: Arc<AtomicBool>,
    /// what the worker was started for (host, sources, excludes, rpo)
    sig: String,
    status: Arc<Mutex<CdpStatus>>,
}

static WORKERS: Mutex<BTreeMap<String, Worker>> = Mutex::new(BTreeMap::new());

fn sig(j: &Job) -> String {
    format!("{}|{}|{}|{}", j.host_id, j.sources.join("\u{1}"), j.excludes.join("\u{1}"), j.cdp_rpo)
}

fn wanted(ctx: &Ctx) -> Vec<Job> {
    all::<Job>(&ctx.db, C_JOBS).into_iter().filter(|j| j.kind == JobKind::File && j.enabled && j.cdp).collect()
}

/// the manager thread
pub fn start(ctx: &Arc<Ctx>) {
    let c = ctx.clone();
    let _ = plat::thread_spawn(128 * 1024, Box::new(move || loop {
        if c.is_set_up() {
            reconcile(&c);
        }
        for _ in 0..4 {
            if plat::service::stop_requested() {
                stop_all();
                return;
            }
            plat::sleep_ms(500);
        }
    }));
}

fn stop_all() {
    for (_, w) in WORKERS.lock().iter() {
        w.stop.store(true, Ordering::SeqCst);
    }
}

fn reconcile(ctx: &Arc<Ctx>) {
    let jobs = wanted(ctx);
    let mut ws = WORKERS.lock();
    // gone, disabled or changed: stop
    let keep: BTreeMap<String, String> = jobs.iter().map(|j| (j.id.clone(), sig(j))).collect();
    ws.retain(|id, w| {
        let ok = keep.get(id).is_some_and(|s| *s == w.sig);
        if !ok {
            w.stop.store(true, Ordering::SeqCst);
        }
        ok
    });
    for j in jobs {
        if ws.contains_key(&j.id) {
            continue;
        }
        let stop = Arc::new(AtomicBool::new(false));
        let status = Arc::new(Mutex::new(CdpStatus { job_id: j.id.clone(), job_name: j.name.clone(), rpo: j.cdp_rpo, keep_hours: j.cdp_keep_hours, state: String::from("starting"), ..CdpStatus::default() }));
        let (c, s2, st2, id) = (ctx.clone(), stop.clone(), status.clone(), j.id.clone());
        if plat::thread_spawn(256 * 1024, Box::new(move || worker(&c, &id, &s2, &st2))).is_some() {
            ws.insert(j.id.clone(), Worker { stop, sig: sig(&j), status });
        }
    }
}

fn set(st: &Mutex<CdpStatus>, f: impl FnOnce(&mut CdpStatus)) {
    f(&mut st.lock());
}

/// the newest point of a job (seconds)
fn last_point(ctx: &Ctx, job: &str) -> i64 {
    all::<RestorePoint>(&ctx.db, C_POINTS).into_iter().filter(|p| p.job_id == job).map(|p| p.created).max().unwrap_or(0)
}

/// sleep in small steps, false when stopped
fn nap(stop: &AtomicBool, ms: u32) -> bool {
    let mut left = ms;
    while left > 0 {
        if stop.load(Ordering::SeqCst) {
            return false;
        }
        let k = left.min(200);
        plat::sleep_ms(k);
        left -= k;
    }
    !stop.load(Ordering::SeqCst)
}

enum Source {
    Local(plat::watch::Watcher),
    Agent { host: Host, since: u64 },
}

fn worker(ctx: &Arc<Ctx>, id: &str, stop: &AtomicBool, st: &Mutex<CdpStatus>) {
    let Some(job): Option<Job> = rec(ctx.get(C_JOBS, id)) else { return };
    let Some(host): Option<Host> = rec(ctx.get(C_HOSTS, &job.host_id)) else {
        set(st, |s| {
            s.state = String::from("error");
            s.message = String::from("Der Host des Jobs fehlt");
        });
        return;
    };
    set(st, |s| s.host_name = host.name.clone());
    // the source of change events (retried until it works)
    let mut src = loop {
        if host.kind == HostKind::Agent {
            break Source::Agent { host: host.clone(), since: u64::MAX };
        }
        match plat::watch::Watcher::new(&job.sources) {
            Ok(w) => break Source::Local(w),
            Err(e) => {
                set(st, |s| {
                    s.state = String::from("error");
                    s.message = e.clone();
                });
                if !nap(stop, 30_000) {
                    return;
                }
            }
        }
    };
    set(st, |s| {
        s.state = String::from("watching");
        s.message.clear();
    });
    let mut pending: BTreeSet<String> = BTreeSet::new();
    // what changed before the watching started is unknown: the first run walks everything
    // (and makes no point when nothing changed)
    let mut overflow = true;
    let mut last_run = 0i64;
    while !stop.load(Ordering::SeqCst) {
        let (paths, lost) = match &mut src {
            Source::Local(w) => {
                let c = w.take(1000);
                (c.paths, c.overflow)
            }
            Source::Agent { host, since } => match crate::agents::changes(ctx, host, *since, 10_000) {
                Ok(c) => {
                    let first = *since == u64::MAX;
                    *since = c.seq;
                    if first { (Vec::new(), false) } else { (c.paths, c.overflow) }
                }
                Err(e) => {
                    set(st, |s| {
                        s.state = String::from("error");
                        s.message = format!("Agent: {}", e);
                    });
                    if !nap(stop, 5000) {
                        return;
                    }
                    continue;
                }
            },
        };
        let t = now();
        let mut seen = lost;
        for p in paths {
            let name = p.rsplit(['/', '\\']).next().unwrap_or(&p);
            if crate::engine::excluded(&job.excludes, name, &p) || name.contains(".bcrestore-") {
                continue;
            }
            pending.insert(p);
            seen = true;
        }
        overflow |= lost;
        if seen {
            set(st, |s| s.last_change = t);
        }
        let busy = ctx.ops.busy(&job.id, "job.run");
        set(st, |s| {
            s.pending = pending.len() as u64;
            s.state = String::from(if busy { "backing-up" } else { "watching" });
            if s.state != "error" {
                s.message.clear();
            }
        });
        let due = t - last_point(ctx, &job.id).max(last_run) >= job.cdp_rpo as i64;
        if (!pending.is_empty() || overflow) && due && !busy {
            let hint = if overflow { None } else { Some(pending.iter().cloned().collect::<Vec<String>>()) };
            let Some(current): Option<Job> = rec(ctx.get(C_JOBS, &job.id)) else { return };
            match crate::engine::start_backup_hint(ctx, &current, false, "CDP", hint) {
                Ok(_) => {
                    pending.clear();
                    overflow = false;
                    last_run = t;
                    set(st, |s| {
                        s.state = String::from("backing-up");
                        s.last_point = t;
                    });
                }
                Err(e) => set(st, |s| s.message = e),
            }
        }
    }
    set(st, |s| s.state = String::from("stopped"));
}

/// GET /api/cdp
pub fn status(ctx: &Ctx) -> Response {
    let ws = WORKERS.lock();
    let t = now();
    let points: Vec<RestorePoint> = all(&ctx.db, C_POINTS);
    let mut out = Vec::new();
    for j in all::<Job>(&ctx.db, C_JOBS).into_iter().filter(|j| j.cdp) {
        let mut s = ws.get(&j.id).map(|w| w.status.lock().clone()).unwrap_or(CdpStatus {
            job_id: j.id.clone(),
            job_name: j.name.clone(),
            state: String::from(if j.enabled { "watching" } else { "stopped" }),
            ..CdpStatus::default()
        });
        s.job_name = j.name.clone();
        s.rpo = j.cdp_rpo;
        s.keep_hours = j.cdp_keep_hours;
        let from = t - j.cdp_keep_hours.max(1) as i64 * 3600;
        let mine: Vec<&RestorePoint> = points.iter().filter(|p| p.job_id == j.id).collect();
        s.points = mine.iter().filter(|p| p.created >= from).count() as u32;
        s.last_point = s.last_point.max(mine.iter().map(|p| p.created).max().unwrap_or(0));
        out.push(s);
    }
    list(&out)
}
