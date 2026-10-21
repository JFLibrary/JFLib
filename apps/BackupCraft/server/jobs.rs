// jobs -- backup jobs and their restore points: create / change (checked against hosts,
// shared folders, repositories and the schedule; the passphrase of an encrypted job is sealed
// with the master key), enable / disable, run now, restore, verify, delete points (not while
// immutable)

use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

use bc_proto::api::{code, JobRequest, RestoreRequest, RunRequest, CAP_HYPERVISOR, CAP_VOLUME};
use bc_proto::j::{self, Json, Value};
use bc_proto::model::*;
use datetime::{Cron, Timestamp, Zone};
use netio::http::Response;

use crate::api::Call;
use crate::ctx::{Ctx, C_HOSTS, C_JOBS, C_POINTS, C_REPOS};
use crate::engine;
use crate::hosts::inside;
use crate::util::{self, all, bad, err, field, list as list_resp, new_id, no_content, not_found, now, ok, rec};

/// the next run of a schedule after `t` (0: none)
pub fn next_run(s: &Schedule, enabled: bool, t: i64) -> i64 {
    if !enabled || !s.enabled || s.cron.trim().is_empty() {
        return 0;
    }
    let zone = Zone::by_name(&s.tz).unwrap_or_else(Zone::utc);
    Cron::parse(&s.cron).ok().and_then(|c| c.next_after(Timestamp::from_secs(t), &zone)).map_or(0, |n| n.secs)
}

pub fn list(ctx: &Ctx) -> Response {
    let mut v: Vec<Job> = all(&ctx.db, C_JOBS);
    v.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    list_resp(&v)
}

pub fn get(ctx: &Ctx, c: &Call) -> Response {
    match rec::<Job>(ctx.get(C_JOBS, &c.param("id"))) {
        Some(j) => ok(200, &j),
        None => not_found("Job nicht gefunden"),
    }
}

fn aad(job: &str) -> String {
    format!("job-passphrase:{}", job)
}

/// the passphrase of an encrypted job (sealed in its record)
pub fn passphrase(ctx: &Ctx, job_id: &str) -> Option<String> {
    let v = ctx.db.get(C_JOBS, job_id)?;
    let sealed = j::str_of(&v, "passphrase");
    if sealed.is_empty() {
        return None;
    }
    String::from_utf8(ctx.keys.unseal(sealed, &aad(job_id))?).ok()
}

pub fn save(ctx: &Ctx, c: &Call) -> Response {
    let Some(r) = JobRequest::from_json(&c.body) else { return bad("Ungültige Anfrage") };
    let id = c.param("id");
    let creating = id.is_empty();
    let old_raw = if creating { None } else { ctx.get(C_JOBS, &id) };
    if !creating && old_raw.is_none() {
        return not_found("Job nicht gefunden");
    }
    let old: Option<Job> = old_raw.as_ref().and_then(Job::from_json);
    // checks
    let name = r.name.trim();
    if name.is_empty() || name.len() > 120 {
        return field(code::BAD_REQUEST, "Bitte einen Namen angeben", "name");
    }
    match r.kind {
        JobKind::File | JobKind::Copy => {}
        JobKind::Tape => {
            if !bc_proto::api::available(bc_proto::api::CAP_TAPE) {
                return err(501, code::CAPABILITY, "tape: in Vorbereitung");
            }
            if !ctx.db.contains(crate::ctx::C_POOLS, &r.pool_id) {
                return field(code::BAD_REQUEST, "Bitte einen Medienpool wählen", "pool_id");
            }
        }
        JobKind::Volume => {
            if !bc_proto::api::available(CAP_VOLUME) {
                return err(501, code::CAPABILITY, &format!("{}: in Vorbereitung", CAP_VOLUME));
            }
            if r.sources.is_empty() {
                return field(code::BAD_REQUEST, "Bitte mindestens ein Volume wählen", "sources");
            }
            if let Some(x) = crate::volume::invalid(&r.sources) {
                return field(code::BAD_REQUEST, &format!("„{}“ ist kein Volume", x), "sources");
            }
            let server = all::<Host>(&ctx.db, C_HOSTS).into_iter().find(|h| h.kind == HostKind::Server).map(|h| h.id).unwrap_or_default();
            if !r.host_id.is_empty() && r.host_id != server {
                return err(501, code::CAPABILITY, "Volumes von Agenten: in Vorbereitung (bisher nur die des Backup-Servers)");
            }
        }
        JobKind::HyperV => return err(501, code::CAPABILITY, &format!("{}: Hyper-V in Vorbereitung", CAP_HYPERVISOR)),
        JobKind::Qemu => {
            if !bc_proto::api::available(CAP_HYPERVISOR) {
                return err(501, code::CAPABILITY, &format!("{}: in Vorbereitung", CAP_HYPERVISOR));
            }
            if r.sources.is_empty() {
                return field(code::BAD_REQUEST, "Bitte mindestens eine VM wählen", "sources");
            }
            if let Some(x) = crate::vm::unknown(ctx, &r.sources) {
                return field(code::BAD_REQUEST, &format!("VM {} ist nicht bekannt", x), "sources");
            }
        }
    }
    if r.kind != JobKind::Tape && !ctx.db.contains(C_REPOS, &r.repository_id) {
        return field(code::BAD_REQUEST, "Bitte ein Repository wählen", "repository_id");
    }
    if r.kind == JobKind::File {
        let Some(host): Option<Host> = rec(ctx.get(C_HOSTS, &r.host_id)) else { return field(code::BAD_REQUEST, "Bitte einen Host wählen", "host_id") };
        let sources: Vec<&String> = r.sources.iter().filter(|s| !s.trim().is_empty()).collect();
        if sources.is_empty() {
            return field(code::BAD_REQUEST, "Bitte mindestens einen Ordner oder eine Datei wählen", "sources");
        }
        if let Some(s) = sources.iter().find(|s| !inside(&host.roots, s)) {
            return field(code::FORBIDDEN, &format!("„{}“ liegt nicht in einem freigegebenen Ordner von {}", s, host.name), "sources");
        }
    } else if matches!(r.kind, JobKind::Copy | JobKind::Tape) {
        if r.source_job.is_empty() || r.source_job == id || !ctx.db.contains(C_JOBS, &r.source_job) {
            return field(code::BAD_REQUEST, "Bitte den Job wählen, dessen Restore-Points kopiert werden", "source_job");
        }
    }
    if !r.schedule.cron.trim().is_empty() {
        if let Err(e) = Cron::parse(&r.schedule.cron) {
            return field(code::BAD_REQUEST, &format!("Ungültiger Zeitplan: {:?}", e), "schedule");
        }
        if Zone::by_name(&r.schedule.tz).is_none() {
            return field(code::BAD_REQUEST, "Unbekannte Zeitzone", "schedule");
        }
    }
    let ret = &r.retention;
    if r.kind != JobKind::Tape && ret.points == 0 && ret.days == 0 && ret.weekly == 0 && ret.monthly == 0 && ret.yearly == 0 {
        return field(code::BAD_REQUEST, "Die Aufbewahrung muss mindestens einen Restore-Point behalten", "retention");
    }
    if r.active_full_days > 365 {
        return field(code::BAD_REQUEST, "Höchstens 365 Tage", "active_full_days");
    }
    if r.cdp && r.kind != JobKind::File {
        return field(code::BAD_REQUEST, "Kontinuierlich sichern lassen sich nur Datei-Jobs", "cdp");
    }
    if r.cdp && !(15..=86_400).contains(&r.cdp_rpo) {
        return field(code::BAD_REQUEST, "Der Abstand der CDP-Punkte liegt zwischen 15 Sekunden und einem Tag", "cdp_rpo");
    }
    if r.cdp_keep_hours > 720 {
        return field(code::BAD_REQUEST, "Höchstens 720 Stunden", "cdp_keep_hours");
    }
    let had_key = old_raw.as_ref().is_some_and(|v| !j::str_of(v, "passphrase").is_empty());
    if r.encrypted && r.kind == JobKind::Tape {
        return field(code::BAD_REQUEST, "Bandjobs verschlüsselt der Medienpool", "encrypted");
    }
    if r.encrypted && r.passphrase.is_empty() && !had_key {
        return field(code::BAD_REQUEST, "Die Passphrase fehlt", "passphrase");
    }
    if !r.passphrase.is_empty() && r.passphrase.chars().count() < 12 {
        return field(code::BAD_REQUEST, "Die Passphrase braucht mindestens 12 Zeichen", "passphrase");
    }
    let t = now();
    let jid = if creating { new_id() } else { id.clone() };
    let base = old.unwrap_or(Job { created: t, last_status: SessionStatus::Never, ..Job::default() });
    let job = Job {
        id: jid.clone(),
        name: String::from(name),
        description: r.description,
        kind: r.kind,
        enabled: r.enabled,
        host_id: match r.kind {
            JobKind::File => r.host_id,
            JobKind::Qemu | JobKind::Volume => all::<Host>(&ctx.db, C_HOSTS).into_iter().find(|h| h.kind == HostKind::Server).map_or(String::new(), |h| h.id),
            _ => String::new(),
        },
        sources: r.sources.into_iter().filter(|s| !s.trim().is_empty()).collect(),
        excludes: r.excludes.into_iter().filter(|s| !s.trim().is_empty()).collect(),
        next_run: next_run(&r.schedule, r.enabled, t),
        schedule: r.schedule,
        retention: r.retention,
        encrypted: r.encrypted,
        key_hint: r.key_hint,
        compression: r.compression,
        active_full_days: r.active_full_days,
        pre_script: r.pre_script,
        post_script: r.post_script,
        source_job: r.source_job,
        pool_id: if r.kind == JobKind::Tape { r.pool_id } else { String::new() },
        repository_id: if r.kind == JobKind::Tape { String::new() } else { r.repository_id },
        cdp: r.cdp,
        cdp_rpo: if r.cdp_rpo == 0 { 60 } else { r.cdp_rpo },
        cdp_keep_hours: r.cdp_keep_hours,
        updated: t,
        ..base
    };
    let mut v = job.to_json();
    let sealed = if !r.passphrase.is_empty() {
        Value::string(ctx.keys.seal(r.passphrase.as_bytes(), &aad(&jid)))
    } else {
        old_raw.as_ref().and_then(|o| o.get("passphrase").cloned()).unwrap_or(Value::string(""))
    };
    if job.encrypted {
        v.insert("passphrase", sealed);
    }
    if ctx.db.put(C_JOBS, &jid, v).is_err() {
        return util::internal("");
    }
    c.audit(if creating { "job.create" } else { "job.update" }, &job.name, if r.passphrase.is_empty() { "" } else { "Neue Passphrase" });
    ok(if creating { 201 } else { 200 }, &job)
}

pub fn delete(ctx: &Ctx, c: &Call) -> Response {
    let id = c.param("id");
    let Some(job): Option<Job> = rec(ctx.get(C_JOBS, &id)) else { return not_found("Job nicht gefunden") };
    if ctx.ops.busy(&id, "job.run") {
        return err(409, code::BUSY, "Der Job läuft gerade");
    }
    if all::<Job>(&ctx.db, C_JOBS).iter().any(|j| j.source_job == id) {
        return err(409, code::CONFLICT, "Ein Kopierjob verwendet diesen Job");
    }
    let _ = ctx.db.delete(C_JOBS, &id);
    c.audit("job.delete", &job.name, "Restore-Points bleiben erhalten");
    no_content()
}

pub fn enable(ctx: &Ctx, c: &Call) -> Response {
    let id = c.param("id");
    let on = c.route.path.ends_with("/enable");
    let mut out = None;
    let r = ctx.db.update(C_JOBS, &id, |v| {
        if let Some(mut job) = Job::from_json(v) {
            job.enabled = on;
            job.next_run = next_run(&job.schedule, on, now());
            job.updated = now();
            let pass = v.get("passphrase").cloned();
            *v = job.to_json();
            if let Some(p) = pass {
                v.insert("passphrase", p);
            }
            out = Some(job);
        }
    });
    match (r, out) {
        (Ok(Some(_)), Some(job)) => {
            c.audit(if on { "job.enable" } else { "job.disable" }, &job.name, "");
            ok(200, &job)
        }
        _ => not_found("Job nicht gefunden"),
    }
}

pub fn run(ctx: &Arc<Ctx>, c: &Call) -> Response {
    let id = c.param("id");
    let Some(job): Option<Job> = rec(ctx.get(C_JOBS, &id)) else { return not_found("Job nicht gefunden") };
    let full = RunRequest::from_json(&c.body).is_some_and(|r| r.full);
    match engine::start_backup(ctx, &job, full, &c.actor()) {
        Ok(op) => {
            c.audit("job.run", &job.name, if full { "Aktive Vollsicherung" } else { "Manuell gestartet" });
            util::accepted(&op)
        }
        Err(e) => err(409, code::BUSY, &e),
    }
}

pub fn points(ctx: &Ctx, c: &Call) -> Response {
    let id = c.param("id");
    let mut v: Vec<RestorePoint> = all::<RestorePoint>(&ctx.db, C_POINTS).into_iter().filter(|p| p.job_id == id).collect();
    v.sort_by_key(|p| core::cmp::Reverse(p.order()));
    list_resp(&v)
}

pub fn point(ctx: &Ctx, c: &Call) -> Response {
    match rec::<RestorePoint>(ctx.get(C_POINTS, &c.param("id"))) {
        Some(p) => ok(200, &p),
        None => not_found("Restore-Point nicht gefunden"),
    }
}

pub fn point_browse(ctx: &Ctx, c: &Call) -> Response {
    let Some(p): Option<RestorePoint> = rec(ctx.get(C_POINTS, &c.param("id"))) else { return not_found("Restore-Point nicht gefunden") };
    let path = c.query("path").unwrap_or_default();
    let offset = c.query("offset").and_then(|o| o.parse().ok()).unwrap_or(0usize);
    let pass = c.query("passphrase");
    match engine::browse_point(ctx, &p, &path, offset, pass.as_deref()) {
        Ok(l) => ok(200, &l),
        Err(e) => err(409, code::CONFLICT, &e),
    }
}

pub fn restore(ctx: &Arc<Ctx>, c: &Call) -> Response {
    let Some(p): Option<RestorePoint> = rec(ctx.get(C_POINTS, &c.param("id"))) else { return not_found("Restore-Point nicht gefunden") };
    let Some(r) = RestoreRequest::from_json(&c.body) else { return bad("Ungültige Anfrage") };
    match engine::start_restore(ctx, &p, r, c.user.as_ref(), &c.actor()) {
        Ok(op) => {
            c.audit("restore", &p.job_name, "");
            util::accepted(&op)
        }
        Err((status, e)) => err(status, if status == 403 { code::FORBIDDEN } else { code::BAD_REQUEST }, &e),
    }
}

pub fn verify(ctx: &Arc<Ctx>, c: &Call) -> Response {
    let Some(p): Option<RestorePoint> = rec(ctx.get(C_POINTS, &c.param("id"))) else { return not_found("Restore-Point nicht gefunden") };
    let op = engine::start_verify(ctx, &p, &c.actor());
    c.audit("point.verify", &p.job_name, "");
    util::accepted(&op)
}

pub fn point_delete(ctx: &Ctx, c: &Call) -> Response {
    let id = c.param("id");
    let Some(p): Option<RestorePoint> = rec(ctx.get(C_POINTS, &id)) else { return not_found("Restore-Point nicht gefunden") };
    if p.immutable_until > now() {
        return err(409, code::IMMUTABLE, "Der Restore-Point ist noch unveränderlich");
    }
    match engine::delete_point(ctx, &p) {
        Ok(()) => {
            c.audit("point.delete", &p.job_name, &format!("{}", p.created));
            no_content()
        }
        Err(e) => err(409, code::CONFLICT, &e),
    }
}

/// start a job from the scheduler
pub fn scheduled(ctx: &Arc<Ctx>, job: &Job) {
    let _ = engine::start_backup(ctx, job, false, "Zeitplan");
}
