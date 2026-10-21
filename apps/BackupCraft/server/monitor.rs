// monitor -- the dashboard summary, sessions and their logs, operations, the audit log

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use bc_proto::api::code;

use bc_proto::model::*;
use netio::http::Response;

use crate::api::Call;
use crate::ctx::{Ctx, C_HOSTS, C_JOBS, C_POINTS, C_SESSIONS};
use crate::util::{all, err, list, not_found, now, ok, rec};

const DAY: i64 = 86_400;

pub fn summary(ctx: &Ctx) -> Response {
    ok(200, &build(ctx))
}

/// the dashboard's summary (also the report a central server reads)
pub fn build(ctx: &Ctx) -> Summary {
    let t = now();
    let jobs: Vec<Job> = all(&ctx.db, C_JOBS);
    let hosts: Vec<Host> = all(&ctx.db, C_HOSTS);
    let repos: Vec<Repository> = crate::repos::all_live(ctx);
    let sessions: Vec<Session> = all(&ctx.db, C_SESSIONS);
    let points: Vec<RestorePoint> = all(&ctx.db, C_POINTS);
    let backups = |s: &&Session| matches!(s.kind, SessionKind::Backup | SessionKind::Copy);
    let recent = |st: SessionStatus| sessions.iter().filter(backups).filter(|s| s.started >= t - DAY && s.status == st).count() as u32;
    let mut history = Vec::new();
    let mut protected_history = Vec::new();
    for d in (0..14).rev() {
        let (a, b) = (t - (d + 1) * DAY, t - d * DAY);
        let (mut okc, mut warn, mut fail) = (0, 0, 0);
        for s in sessions.iter().filter(backups).filter(|s| s.started >= a && s.started < b) {
            match s.status {
                SessionStatus::Success => okc += 1,
                SessionStatus::Warning => warn += 1,
                SessionStatus::Failed => fail += 1,
                _ => {}
            }
        }
        history.push(DayStat { day: b, ok: okc, warn, fail });
        // the protected bytes then: per job the size of its newest point before b
        let mut sum = 0u64;
        for j in &jobs {
            if let Some(p) = points.iter().filter(|p| p.job_id == j.id && p.created < b).max_by_key(|p| p.created) {
                sum += p.size;
            }
        }
        protected_history.push(sum);
    }
    let mut next: Vec<NextRun> = jobs.iter().filter(|j| j.enabled && j.next_run > 0).map(|j| NextRun { job_id: j.id.clone(), job_name: j.name.clone(), at: j.next_run }).collect();
    next.sort_by_key(|n| n.at);
    next.truncate(5);
    let mut alerts = Vec::new();
    for j in &jobs {
        let last = sessions.iter().filter(|s| s.job_id == j.id && s.kind == SessionKind::Backup).max_by_key(|s| s.started);
        let detail = last.map_or(String::new(), |s| s.message.clone());
        match j.last_status {
            SessionStatus::Failed => alerts.push(Alert { severity: Severity::Critical, title: format!("{} ist fehlgeschlagen", j.name), detail, target_kind: String::from("job"), target_id: j.id.clone(), at: j.last_run }),
            SessionStatus::Warning => alerts.push(Alert { severity: Severity::Warning, title: format!("{} mit Warnungen", j.name), detail, target_kind: String::from("job"), target_id: j.id.clone(), at: j.last_run }),
            _ => {}
        }
    }
    for r in &repos {
        // the disk counts (other data on it takes room too), not only the repository
        if r.status != RepoStatus::Offline && r.capacity > 0 && r.free.saturating_mul(10) < r.capacity {
            let detail = format!("{} % belegt, noch {} frei", r.disk_used().saturating_mul(100) / r.capacity, crate::engine::size(r.free));
            alerts.push(Alert { severity: Severity::Warning, title: format!("Repository „{}“ fast voll", r.name), detail, target_kind: String::from("repository"), target_id: r.id.clone(), at: t });
        }
        if r.status == RepoStatus::Offline || r.status == RepoStatus::Error {
            alerts.push(Alert { severity: Severity::Critical, title: format!("Repository „{}“ nicht erreichbar", r.name), detail: r.message.clone(), target_kind: String::from("repository"), target_id: r.id.clone(), at: t });
        }
    }
    for h in &hosts {
        if h.status == HostStatus::Offline {
            alerts.push(Alert { severity: Severity::Critical, title: format!("{} ist offline", h.name), detail: h.message.clone(), target_kind: String::from("host"), target_id: h.id.clone(), at: h.last_seen });
        }
    }
    alerts.extend(crate::fleet::alerts(ctx));
    alerts.sort_by(|a, b| b.at.cmp(&a.at));
    // a newer version comes last: something to plan, not a fault
    alerts.extend(crate::update::alert(ctx));
    Summary {
        server_name: ctx.server_name(),
        version: String::from(bc_proto::VERSION),
        jobs: jobs.len() as u32,
        jobs_enabled: jobs.iter().filter(|j| j.enabled).count() as u32,
        hosts: hosts.len() as u32,
        hosts_online: hosts.iter().filter(|h| h.status == HostStatus::Online).count() as u32,
        protected_items: jobs.iter().map(|j| j.sources.len() as u32).sum(),
        points: points.len() as u32,
        ok_24h: recent(SessionStatus::Success),
        warn_24h: recent(SessionStatus::Warning),
        fail_24h: recent(SessionStatus::Failed),
        running: ctx.ops.running(),
        storage_used: repos.iter().map(|r| r.used).sum(),
        storage_capacity: bc_proto::model::disks(&repos).0,
        storage_free: bc_proto::model::disks(&repos).1,
        storage_logical: repos.iter().map(|r| r.logical).sum(),
        history,
        next_runs: next,
        alerts,
        protected_history,
    }
}

pub fn sessions(ctx: &Ctx, c: &Call) -> Response {
    let limit = c.query("limit").and_then(|l| l.parse::<usize>().ok()).unwrap_or(500).min(5000);
    let job = c.query("job").unwrap_or_default();
    let mut v: Vec<Session> = all::<Session>(&ctx.db, C_SESSIONS).into_iter().filter(|s| job.is_empty() || s.job_id == job).collect();
    v.sort_by(|a, b| b.started.cmp(&a.started));
    v.truncate(limit);
    list(&v)
}

pub fn session(ctx: &Ctx, c: &Call) -> Response {
    match rec::<Session>(ctx.get(C_SESSIONS, &c.param("id"))) {
        Some(s) => ok(200, &s),
        None => not_found("Sitzung nicht gefunden"),
    }
}

pub fn session_log(ctx: &Ctx, c: &Call) -> Response {
    let id = c.param("id");
    if !ctx.db.contains(C_SESSIONS, &id) {
        return not_found("Sitzung nicht gefunden");
    }
    list(&ctx.session_log(&id))
}

pub fn operations(ctx: &Ctx) -> Response {
    list(&ctx.ops.list(&ctx.db, 300))
}

pub fn operation(ctx: &Ctx, c: &Call) -> Response {
    match ctx.ops.get(&ctx.db, &c.param("id")) {
        Some(o) => ok(200, &o),
        None => not_found("Vorgang nicht gefunden"),
    }
}

pub fn cancel(ctx: &Ctx, c: &Call) -> Response {
    let id = c.param("id");
    match ctx.ops.cancel(&id) {
        Some(o) => {
            c.audit("operation.cancel", &o.title, "");
            ok(200, &o)
        }
        None if ctx.ops.get(&ctx.db, &id).is_some() => err(409, code::CONFLICT, "Der Vorgang ist bereits beendet"),
        None => not_found("Vorgang nicht gefunden"),
    }
}

pub fn audit(ctx: &Ctx, c: &Call) -> Response {
    let limit = c.query("limit").and_then(|l| l.parse::<usize>().ok()).unwrap_or(500).min(10_000);
    list(&ctx.audit_tail(limit))
}
