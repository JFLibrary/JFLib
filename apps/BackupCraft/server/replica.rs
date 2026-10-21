// replica -- standby copies of a job's newest restore point in a folder of a host, failover
// plans
//
//   list / save / delete            replicas: a job, a host (the server or an agent), a folder
//                                   inside that host's restore folders
//   start_sync(ctx, replica, ..)    mirror the job's newest point into the folder (unchanged
//                                   files stay, changed ones are replaced, what the point lacks
//                                   goes); auto replicas follow every new point (after_backup)
//   failover                        the replica is used instead of the original: syncing stops
//   failback                        the replica's folders are backed up (a point of the job),
//                                   then written back to the original places (changed files
//                                   replaced; nothing is deleted at the original), the replica
//                                   is ready again
//   undo                            the failover is dropped: the replica goes back to its point
//   plans                           replicas switched over in order with a pause between them
//
// Each root R of the job lands in <folder>/<R without drive or leading separator>, as in an
// alternative restore.

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

use bc_proto::api::{code, PlanRequest, ReplicaRequest};
use bc_proto::j::{Json, Value};
use bc_proto::model::*;
use netio::http::Response;

use crate::api::Call;
use crate::ctx::{Ctx, C_HOSTS, C_JOBS, C_PLANS, C_POINTS, C_REPLICAS};
use crate::engine::{self, size};
use crate::ops::{OpCtl, Ops};
use crate::restore::{allowed_roots, mirror, relative_of};
use crate::util::{self, all, err, field, list as list_resp, new_id, no_content, not_found, now, ok, rec};

fn get(ctx: &Ctx, id: &str) -> Option<Replica> {
    rec(ctx.get(C_REPLICAS, id))
}

fn put(ctx: &Ctx, r: &Replica) {
    let _ = ctx.db.put(C_REPLICAS, &r.id, r.to_json());
}

/// change a replica record
fn set(ctx: &Ctx, id: &str, f: impl FnOnce(&mut Replica)) -> Option<Replica> {
    let mut r = get(ctx, id)?;
    f(&mut r);
    put(ctx, &r);
    Some(r)
}

pub fn list(ctx: &Ctx) -> Response {
    let mut v: Vec<Replica> = all(&ctx.db, C_REPLICAS);
    v.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    list_resp(&v)
}

/// the newest restore point of a job
fn newest(ctx: &Ctx, job_id: &str) -> Option<RestorePoint> {
    all::<RestorePoint>(&ctx.db, C_POINTS).into_iter().filter(|p| p.job_id == job_id).max_by_key(|p| p.order())
}

pub fn save(ctx: &Ctx, c: &Call) -> Response {
    let Some(r) = ReplicaRequest::from_json(&c.body) else { return util::bad("Ungültige Anfrage") };
    let id = c.param("id");
    let old = if id.is_empty() { None } else { get(ctx, &id) };
    if !id.is_empty() && old.is_none() {
        return not_found("Replikat nicht gefunden");
    }
    let name = r.name.trim();
    if name.is_empty() || name.len() > 120 {
        return field(code::BAD_REQUEST, "Bitte einen Namen angeben", "name");
    }
    let Some(job): Option<Job> = rec(ctx.get(C_JOBS, &r.job_id)) else { return field(code::BAD_REQUEST, "Bitte einen Job wählen", "job_id") };
    let Some(host): Option<Host> = rec(ctx.get(C_HOSTS, &r.host_id)) else { return field(code::BAD_REQUEST, "Bitte einen Zielhost wählen", "host_id") };
    let win = crate::hosts::host_windows(&host);
    let target = String::from(r.target.trim());
    if !bc_fs::absolute(&target, win) {
        return field(code::BAD_REQUEST, "Bitte einen absoluten Zielordner angeben", "target");
    }
    if !bc_fs::inside_os(&allowed_roots(ctx, &host), &target, win) {
        return field(code::FORBIDDEN, &format!("„{}“ liegt nicht in einem Wiederherstellungsordner von {}", target, host.name), "target");
    }
    if job.host_id == host.id && job.sources.iter().any(|s| bc_fs::inside_os(&[s.clone()], &target, win) || bc_fs::inside_os(&[target.clone()], s, win)) {
        return field(code::BAD_REQUEST, "Das Replikat darf nicht in den Quellen des Jobs liegen", "target");
    }
    if all::<Replica>(&ctx.db, C_REPLICAS).iter().any(|x| x.id != id && x.host_id == host.id && x.target.eq_ignore_ascii_case(&target)) {
        return field(code::CONFLICT, "In diesen Ordner repliziert schon ein anderes Replikat", "target");
    }
    let t = now();
    let rep = match old {
        Some(o) => {
            if o.state == ReplicaState::Syncing || o.state == ReplicaState::Failback {
                return err(409, code::BUSY, "Das Replikat wird gerade abgeglichen");
            }
            let moved = o.host_id != host.id || o.target != target || o.job_id != job.id;
            Replica {
                name: String::from(name),
                job_id: job.id.clone(),
                job_name: job.name.clone(),
                host_id: host.id.clone(),
                host_name: host.name.clone(),
                target,
                auto: r.auto,
                state: if moved { ReplicaState::New } else { o.state },
                point_id: if moved { String::new() } else { o.point_id.clone() },
                ..o
            }
        }
        None => Replica {
            id: new_id(),
            name: String::from(name),
            job_id: job.id.clone(),
            job_name: job.name.clone(),
            host_id: host.id.clone(),
            host_name: host.name.clone(),
            target,
            auto: r.auto,
            state: ReplicaState::New,
            created: t,
            ..Replica::default()
        },
    };
    put(ctx, &rep);
    c.audit(if id.is_empty() { "replica.create" } else { "replica.update" }, &rep.name, &format!("{} → {}:{}", job.name, host.name, rep.target));
    ok(if id.is_empty() { 201 } else { 200 }, &rep)
}

pub fn delete(ctx: &Ctx, c: &Call) -> Response {
    let id = c.param("id");
    let Some(r) = get(ctx, &id) else { return not_found("Replikat nicht gefunden") };
    if matches!(r.state, ReplicaState::Syncing | ReplicaState::Failback) {
        return err(409, code::BUSY, "Das Replikat wird gerade abgeglichen");
    }
    if all::<FailoverPlan>(&ctx.db, C_PLANS).iter().any(|p| p.replicas.contains(&id)) {
        return err(409, code::CONFLICT, "Ein Failover-Plan verwendet dieses Replikat");
    }
    let _ = ctx.db.delete(C_REPLICAS, &id);
    c.audit("replica.delete", &r.name, "Die Dateien im Zielordner bleiben");
    no_content()
}

/// the (node, destination) pairs of a point below a replica folder
fn pairs(snap: &cas::Snapshot, target: &str, win: bool) -> Vec<(cas::Node, String)> {
    snap.roots.iter().map(|n| (n.clone(), bc_fs::join_os(target, &relative_of(&n.name, win), win))).collect()
}

/// mirror a point into the replica's folder (a session of the job)
fn sync(o: &OpCtl, ctx: &Ctx, rep: &Replica, p: &RestorePoint, s: &mut Session) -> Result<String, String> {
    let host: Host = rec(ctx.get(C_HOSTS, &rep.host_id)).ok_or("Der Zielhost des Replikats fehlt")?;
    if host.kind == HostKind::Agent {
        crate::agents::info(ctx, &host).map_err(|e| format!("Agent nicht erreichbar: {}", e))?;
    }
    let (_, repo) = engine::repo_for(ctx, &p.repository_id)?;
    let snap = repo.load_snapshot(&p.id).map_err(engine::err_text)?;
    let win = crate::hosts::host_windows(&host);
    let roots = allowed_roots(ctx, &host);
    let root = bc_fs::root_of(&roots, &rep.target).map(String::from);
    engine::session_log(ctx, s, LogLevel::Info, &format!("Replikat „{}“: Restore-Point vom {} nach {}:{}", rep.name, crate::notify::when(p.created), host.name, rep.target));
    let m = mirror(o, ctx, &repo, &host, root, &pairs(&snap, &rep.target, win), true, s)?;
    if o.cancelled() {
        return Err(String::from("Abgebrochen"));
    }
    s.files_total = m.files;
    s.files_done = m.files;
    s.bytes_read = m.bytes;
    set(ctx, &rep.id, |r| {
        r.point_id = p.id.clone();
        r.point_time = p.created;
        r.synced = now();
        r.files = m.files;
        r.bytes = m.bytes;
    });
    Ok(format!("{} Dateien geschrieben, {} unverändert, {} entfernt ({})", m.written, m.unchanged, m.removed, size(m.bytes)))
}

/// sync a replica to the job's newest point (or, undo, back to the point it shows)
pub fn start_sync(ctx: &Arc<Ctx>, rep: &Replica, actor: &str, undo: bool) -> Result<String, (u16, String)> {
    if matches!(rep.state, ReplicaState::Syncing | ReplicaState::Failback) || ctx.ops.busy(&rep.id, "replica.sync") {
        return Err((409, String::from("Das Replikat wird gerade abgeglichen")));
    }
    if rep.state == ReplicaState::Failover && !undo {
        return Err((409, String::from("Das Replikat ist im Failover: erst Failback oder Verwerfen")));
    }
    if undo && rep.state != ReplicaState::Failover {
        return Err((409, String::from("Das Replikat ist nicht im Failover")));
    }
    let p = if undo { rec::<RestorePoint>(ctx.get(C_POINTS, &rep.point_id)) } else { newest(ctx, &rep.job_id) }
        .ok_or((409, String::from("Für diesen Job gibt es noch keinen Restore-Point")))?;
    let job = rec::<Job>(ctx.get(C_JOBS, &rep.job_id)).unwrap_or(Job { id: rep.job_id.clone(), name: rep.job_name.clone(), ..Job::default() });
    let before = rep.state;
    set(ctx, &rep.id, |r| r.state = ReplicaState::Syncing);
    let mut session = engine::session_for(ctx, SessionKind::Replica, &job, &p.id);
    let rep = rep.clone();
    let title = format!("Replikat {}: {}", rep.name, if undo { "Failover verwerfen" } else { "Abgleich" });
    Ok(Ops::start(ctx, "replica.sync", &title, actor, &rep.id.clone(), &session.id.clone(), Box::new(move |o| {
        let ctx = o.ctx().clone();
        let r = sync(o, &ctx, &rep, &p, &mut session);
        engine::finish_session(&ctx, &mut session, &r, o.cancelled());
        set(&ctx, &rep.id, |x| {
            match &r {
                Ok(m) => {
                    x.state = ReplicaState::Ready;
                    x.message = m.clone();
                    x.failover_at = 0;
                }
                Err(e) => {
                    // a cancelled undo stays in failover; anything else needs a new sync
                    x.state = if undo && o.cancelled() { before } else { ReplicaState::Failed };
                    x.message = e.clone();
                }
            }
        });
        r
    })))
}

pub fn sync_now(ctx: &Arc<Ctx>, c: &Call) -> Response {
    let Some(rep) = get(ctx, &c.param("id")) else { return not_found("Replikat nicht gefunden") };
    let undo = c.route.path.ends_with("/undo");
    match start_sync(ctx, &rep, &c.actor(), undo) {
        Ok(op) => {
            c.audit(if undo { "replica.undo" } else { "replica.sync" }, &rep.name, "");
            util::accepted(&op)
        }
        Err((st, e)) => err(st, if st == 409 { code::BUSY } else { code::BAD_REQUEST }, &e),
    }
}

/// switch to the replica (no sync from now on)
fn do_failover(ctx: &Ctx, rep: &Replica) -> Result<Replica, String> {
    match rep.state {
        ReplicaState::Ready => {}
        ReplicaState::Failover => return Err(String::from("Das Replikat ist schon im Failover")),
        ReplicaState::New => return Err(String::from("Das Replikat wurde noch nie abgeglichen")),
        _ => return Err(format!("Das Replikat ist nicht bereit ({})", rep.state.as_str())),
    }
    let t = now();
    set(ctx, &rep.id, |r| {
        r.state = ReplicaState::Failover;
        r.failover_at = t;
        r.message = format!("Failover seit {}: {} auf {} ist in Gebrauch", crate::notify::when(t), r.target, r.host_name);
    })
    .ok_or_else(|| String::from("Replikat nicht gefunden"))
}

pub fn failover(ctx: &Ctx, c: &Call) -> Response {
    let Some(rep) = get(ctx, &c.param("id")) else { return not_found("Replikat nicht gefunden") };
    match do_failover(ctx, &rep) {
        Ok(r) => {
            c.audit("replica.failover", &rep.name, &format!("{}:{}", rep.host_name, rep.target));
            ok(200, &r)
        }
        Err(e) => err(409, code::CONFLICT, &e),
    }
}

/// back up the replica's folders, write them back to the original places
fn failback(o: &OpCtl, ctx: &Arc<Ctx>, rep: &Replica, s: &mut Session) -> Result<String, String> {
    let job: Job = rec(ctx.get(C_JOBS, &rep.job_id)).ok_or("Der Job des Replikats fehlt")?;
    let rhost: Host = rec(ctx.get(C_HOSTS, &rep.host_id)).ok_or("Der Zielhost des Replikats fehlt")?;
    let ohost: Host = rec(ctx.get(C_HOSTS, &job.host_id)).ok_or("Der Originalhost des Jobs fehlt")?;
    let repo_rec: Repository = rec(ctx.get(crate::ctx::C_REPOS, &job.repository_id)).ok_or("Das Repository des Jobs fehlt")?;
    let base: RestorePoint = rec(ctx.get(C_POINTS, &rep.point_id)).ok_or("Der Restore-Point des Replikats fehlt")?;
    let (_, base_repo) = engine::repo_for(ctx, &base.repository_id)?;
    let base_snap = base_repo.load_snapshot(&base.id).map_err(engine::err_text)?;
    let rwin = crate::hosts::host_windows(&rhost);
    // 1. the replica's folders as a point of the job
    let sources: Vec<String> = base_snap.roots.iter().map(|n| bc_fs::join_os(&rep.target, &relative_of(&n.name, rwin), rwin)).collect();
    engine::session_log(ctx, s, LogLevel::Info, &format!("Failback „{}“: die Ordner des Replikats werden gesichert", rep.name));
    let fb_job = Job { sources: sources.clone(), excludes: Vec::new(), ..job.clone() };
    let names: Vec<String> = base_snap.roots.iter().map(|n| n.name.clone()).collect();
    let point_id = engine::backup_folders(o, ctx, &fb_job, &rhost, &repo_rec, &allowed_roots(ctx, &rhost), &names, s)?;
    let p: RestorePoint = rec(ctx.get(C_POINTS, &point_id)).ok_or("Der Restore-Point des Failbacks fehlt")?;
    let (_, repo) = engine::repo_for(ctx, &p.repository_id)?;
    let snap = repo.load_snapshot(&p.id).map_err(engine::err_text)?;
    // 2. back to the original places (replacing changed files, deleting nothing)
    if ohost.kind == HostKind::Agent {
        crate::agents::info(ctx, &ohost).map_err(|e| format!("Originalhost nicht erreichbar: {}", e))?;
    }
    // the point names its roots like the job: back to where they came from
    let back: Vec<(cas::Node, String)> = snap.roots.iter().map(|n| (n.clone(), n.name.clone())).collect();
    engine::session_log(ctx, s, LogLevel::Info, &format!("Failback: zurück nach {} ({} Quellen)", ohost.name, back.len()));
    let m = mirror(o, ctx, &repo, &ohost, None, &back, false, s)?;
    set(ctx, &rep.id, |r| {
        r.point_id = p.id.clone();
        r.point_time = p.created;
    });
    Ok(format!("Failback abgeschlossen: {} Dateien zurückgeschrieben, {} unverändert ({}); am Original wurde nichts gelöscht", m.written, m.unchanged, size(m.bytes)))
}

pub fn failback_now(ctx: &Arc<Ctx>, c: &Call) -> Response {
    let Some(rep) = get(ctx, &c.param("id")) else { return not_found("Replikat nicht gefunden") };
    if rep.state != ReplicaState::Failover {
        return err(409, code::CONFLICT, "Failback nur aus dem Failover");
    }
    if ctx.ops.busy(&rep.job_id, "job.run") {
        return err(409, code::BUSY, "Der Job des Replikats läuft gerade");
    }
    let job = rec::<Job>(ctx.get(C_JOBS, &rep.job_id)).unwrap_or(Job { id: rep.job_id.clone(), name: rep.job_name.clone(), ..Job::default() });
    set(ctx, &rep.id, |r| r.state = ReplicaState::Failback);
    let mut session = engine::session_for(ctx, SessionKind::Failback, &job, "");
    let rep2 = rep.clone();
    let op = Ops::start(ctx, "replica.failback", &format!("Replikat {}: Failback", rep.name), &c.actor(), &rep.id, &session.id.clone(), Box::new(move |o| {
        let ctx = o.ctx().clone();
        let r = failback(o, &ctx, &rep2, &mut session);
        engine::finish_session(&ctx, &mut session, &r, o.cancelled());
        set(&ctx, &rep2.id, |x| match &r {
            Ok(m) => {
                x.state = ReplicaState::Ready;
                x.failover_at = 0;
                x.synced = now();
                x.message = m.clone();
            }
            Err(e) => {
                x.state = ReplicaState::Failover;
                x.message = format!("Failback fehlgeschlagen: {}", e);
            }
        });
        r
    }));
    c.audit("replica.failback", &rep.name, "");
    util::accepted(&op)
}

/// replicas that follow their job: a sync after each new point
pub fn after_backup(ctx: &Ctx, job: &Job) {
    let Some(arc) = crate::ctx::current() else { return };
    for r in all::<Replica>(&ctx.db, C_REPLICAS) {
        if r.auto && r.job_id == job.id && matches!(r.state, ReplicaState::Ready | ReplicaState::New | ReplicaState::Failed) {
            let _ = start_sync(&arc, &r, "Nach Sicherung", false);
        }
    }
}

// ------------------------------------------------------------------ failover plans

pub fn plans(ctx: &Ctx) -> Response {
    let mut v: Vec<FailoverPlan> = all(&ctx.db, C_PLANS);
    v.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    list_resp(&v)
}

pub fn plan_save(ctx: &Ctx, c: &Call) -> Response {
    let Some(r) = PlanRequest::from_json(&c.body) else { return util::bad("Ungültige Anfrage") };
    let id = c.param("id");
    let old: Option<FailoverPlan> = if id.is_empty() { None } else { rec(ctx.get(C_PLANS, &id)) };
    if !id.is_empty() && old.is_none() {
        return not_found("Failover-Plan nicht gefunden");
    }
    let name = r.name.trim();
    if name.is_empty() || name.len() > 120 {
        return field(code::BAD_REQUEST, "Bitte einen Namen angeben", "name");
    }
    if r.replicas.is_empty() {
        return field(code::BAD_REQUEST, "Bitte mindestens ein Replikat wählen", "replicas");
    }
    if let Some(x) = r.replicas.iter().find(|x| !ctx.db.contains(C_REPLICAS, x)) {
        return field(code::BAD_REQUEST, &format!("Replikat {} nicht gefunden", x), "replicas");
    }
    if r.delay_s > 3600 {
        return field(code::BAD_REQUEST, "Höchstens 3600 Sekunden Pause", "delay_s");
    }
    let plan = FailoverPlan {
        id: if id.is_empty() { new_id() } else { id.clone() },
        name: String::from(name),
        replicas: r.replicas,
        delay_s: r.delay_s,
        created: old.as_ref().map_or(now(), |o| o.created),
        ..old.unwrap_or_default()
    };
    let _ = ctx.db.put(C_PLANS, &plan.id, plan.to_json());
    c.audit(if id.is_empty() { "plan.create" } else { "plan.update" }, &plan.name, "");
    ok(if id.is_empty() { 201 } else { 200 }, &plan)
}

pub fn plan_delete(ctx: &Ctx, c: &Call) -> Response {
    let id = c.param("id");
    let Some(p): Option<FailoverPlan> = rec(ctx.get(C_PLANS, &id)) else { return not_found("Failover-Plan nicht gefunden") };
    let _ = ctx.db.delete(C_PLANS, &id);
    c.audit("plan.delete", &p.name, "");
    no_content()
}

pub fn plan_run(ctx: &Arc<Ctx>, c: &Call) -> Response {
    let id = c.param("id");
    let Some(plan): Option<FailoverPlan> = rec(ctx.get(C_PLANS, &id)) else { return not_found("Failover-Plan nicht gefunden") };
    if ctx.ops.busy(&id, "plan.run") {
        return err(409, code::BUSY, "Der Plan läuft bereits");
    }
    let p2 = plan.clone();
    let op = Ops::start(ctx, "plan.run", &format!("Failover-Plan {}", plan.name), &c.actor(), &id, "", Box::new(move |o| {
        let ctx = o.ctx().clone();
        let n = p2.replicas.len() as u64;
        let mut done = Vec::new();
        let mut failed = Vec::new();
        for (i, rid) in p2.replicas.iter().enumerate() {
            if o.cancelled() {
                break;
            }
            match get(&ctx, rid) {
                Some(r) => match do_failover(&ctx, &r) {
                    Ok(_) => done.push(r.name.clone()),
                    Err(e) => failed.push(format!("{}: {}", r.name, e)),
                },
                None => failed.push(format!("{}: nicht gefunden", rid)),
            }
            o.progress(i as u64 + 1, n);
            if i + 1 < p2.replicas.len() {
                for _ in 0..p2.delay_s * 10 {
                    if o.cancelled() {
                        break;
                    }
                    plat::sleep_ms(100);
                }
            }
        }
        let msg = if failed.is_empty() { format!("{} Replikate umgeschaltet", done.len()) } else { format!("{} umgeschaltet, {} nicht: {}", done.len(), failed.len(), failed.join("; ")) };
        let ok_ = failed.is_empty() && !o.cancelled();
        let _ = ctx.db.update(C_PLANS, &p2.id, |v| {
            v.insert("last_run", Value::i64(now()));
            v.insert("last_status", Value::string(if ok_ { "success" } else { "failed" }));
            v.insert("message", Value::string(msg.as_str()));
        });
        if ok_ { Ok(msg) } else { Err(msg) }
    }));
    c.audit("plan.run", &plan.name, "");
    util::accepted(&op)
}
