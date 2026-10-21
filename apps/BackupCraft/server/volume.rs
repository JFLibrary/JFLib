// volume -- whole volumes of the backup server block by block (JobKind::Volume): a VSS shadow
// copy (Windows) or an LVM snapshot (Linux) is read in blocks of bc_vm::BLOCK; blocks that did
// not change since the last point are stored once (dedupe), so later points cost little space
// even without change tracking
//
//   volumes(ctx, c)                     GET /api/hosts/{id}/volumes (the server's own)
//   start_job(ctx, job, actor)          job.sources are volume names ("C:", /dev/vg/lv, an
//                                       image file)
//
// A volume in a point is a folder named like the volume: volume.json (size, file system,
// label, how it was read) and volume.raw, the image of the whole volume (restored as a file;
// written back with dd or attached as a disk image).  VSS and LVM need administrator / root
// rights; without a snapshot a volume is read as it is (the session says so).

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

use bc_proto::j::{self, Json, Value};
use bc_proto::model::*;
use cas::{Node, NodeKind, Snapshot, Tree};
use netio::http::Response;

use crate::api::Call;
use crate::ctx::{Ctx, C_HOSTS, C_POINTS, C_REPOS};
use crate::engine::{self, size};
use crate::ops::{OpCtl, Ops};
use crate::util::{all, err, list, new_id, not_found, now, rec};
use crate::vm::{file_node, ingest_file, json_node, Ingest};

const DAY: i64 = 86_400;

fn server_host(ctx: &Ctx) -> Option<Host> {
    all::<Host>(&ctx.db, C_HOSTS).into_iter().find(|h| h.kind == HostKind::Server)
}

pub fn volumes(ctx: &Ctx, c: &Call) -> Response {
    let Some(h): Option<Host> = rec(ctx.get(C_HOSTS, &c.param("id"))) else { return not_found("Host nicht gefunden") };
    if h.kind != HostKind::Server {
        return err(501, bc_proto::api::code::CAPABILITY, "Volumes von Agenten: in Vorbereitung (bisher nur die des Backup-Servers)");
    }
    match bc_volume::list() {
        Ok(v) => list(&v.into_iter().map(|x| Volume { id: x.id, label: x.label, fs: x.fs, size: x.size, free: x.free, kind: x.kind, mount: x.mount }).collect::<Vec<_>>()),
        Err(e) => err(500, bc_proto::api::code::INTERNAL, &e),
    }
}

fn backup_volume(o: &OpCtl, ctx: &Ctx, w: &mut cas::Writer, id: &str, s: &mut Session, st: &mut Ingest) -> Result<Node, String> {
    let tag: String = new_id().chars().take(8).collect();
    let snap = bc_volume::snapshot(id, &tag)?;
    let how = match snap.method.as_str() {
        "" => String::from("ohne Snapshot (wie es ist)"),
        m => format!("über einen {}-Snapshot", m),
    };
    engine::session_log(ctx, s, if snap.method.is_empty() { LogLevel::Warning } else { LogLevel::Info }, &format!("{}: {} ({})", id, how, size(snap.size)));
    if snap.method.is_empty() && !plat::fs::stat(id).is_some_and(|m| m.is_file()) {
        s.warnings += 1;
    }
    st.total = st.total.max(st.done + snap.size);
    let before = w.stats.new_chunks;
    let r = snap.open().and_then(|f| ingest_file(o, w, "volume.raw", &f, &snap.device, snap.size, st));
    if let Err(e) = snap.release() {
        s.warnings += 1;
        engine::session_log(ctx, s, LogLevel::Warning, &format!("{}: der Snapshot ließ sich nicht entfernen: {}", id, e));
    }
    let raw = r?;
    let blocks = raw.chunks.len();
    engine::session_log(ctx, s, LogLevel::Info, &format!("{}: {} von {} Blöcken neu gespeichert", id, w.stats.new_chunks - before, blocks));
    let info = bc_volume::list().ok().and_then(|l| l.into_iter().find(|v| v.id == id));
    let cfg = j::obj([
        ("volume", Value::string(id)),
        ("size", Value::u64(snap.size)),
        ("fs", Value::string(info.as_ref().map_or("", |v| v.fs.as_str()))),
        ("label", Value::string(info.as_ref().map_or("", |v| v.label.as_str()))),
        ("snapshot", Value::string(snap.method.as_str())),
        ("block", Value::u64(bc_vm::BLOCK)),
    ]);
    let meta = json_node(w, "volume.json", &cfg)?;
    let tid = w.put_tree(&Tree { nodes: alloc::vec![meta, raw] }).map_err(engine::err_text)?;
    let mut n = file_node(id, snap.size, Vec::new());
    n.kind = NodeKind::Dir;
    n.tree = Some(tid);
    n.count = 2;
    n.mode = 0o700;
    Ok(n)
}

fn run_backup(o: &OpCtl, ctx: &Ctx, job: &Job, r: &Repository, s: &mut Session) -> Result<String, String> {
    let repo = engine::open_repo(ctx, r)?;
    engine::ensure_job_key(ctx, job, &repo, s)?;
    let prev = engine::previous(ctx, job, &r.id);
    let (codec, level) = engine::codec_of(job.compression);
    let mut w = repo.writer().codec(codec, level);
    let mut st = Ingest { done: 0, total: 0, files: 0 };
    let mut roots = Vec::new();
    for id in &job.sources {
        match backup_volume(o, ctx, &mut w, id, s, &mut st) {
            Ok(n) => roots.push(n),
            Err(e) if e == "Abgebrochen" => return Err(e),
            Err(e) => {
                s.errors += 1;
                engine::session_log(ctx, s, LogLevel::Error, &format!("{}: {}", id, e));
            }
        }
    }
    let ws = w.finish().map_err(engine::err_text)?;
    if roots.is_empty() {
        return Err(String::from("Kein Volume wurde gesichert"));
    }
    s.files_total = st.files;
    s.files_done = st.files;
    s.bytes_total = st.done;
    s.bytes_read = st.done;
    s.bytes_new = ws.new_stored;
    if ws.new_stored > 0 {
        s.dedup_x100 = (ws.raw.saturating_mul(100) / ws.new_raw.max(1)).min(u32::MAX as u64) as u32;
        s.compress_x100 = (ws.new_raw.saturating_mul(100) / ws.new_stored.max(1)).min(u32::MAX as u64) as u32;
    }
    let t = now();
    let imm = repo.immutable_days();
    let bytes: u64 = roots.iter().map(|n| n.size).sum();
    let snap = Snapshot {
        id: new_id(),
        created: t,
        job_id: job.id.clone(),
        job_name: job.name.clone(),
        host: server_host(ctx).map_or(String::new(), |h| h.name),
        roots,
        parent: prev.as_ref().map_or(String::new(), |p| p.id.clone()),
        full: true,
        stats: cas::SnapStats { files: st.files, dirs: 0, bytes, new_bytes: ws.new_stored, new_chunks: ws.new_chunks, new_raw: ws.new_raw, errors: s.errors as u64 },
        immutable_until: if imm > 0 { t + imm as i64 * DAY } else { 0 },
        meta: j::obj([("session", Value::string(s.id.as_str())), ("created_ns", Value::i64(plat::unix_time_ns())), ("kind", Value::string("volume"))]),
    };
    repo.save_snapshot(&snap, &ctx.keys.signing).map_err(engine::err_text)?;
    let point = engine::point_record(&snap, &r.id, job.encrypted);
    let _ = ctx.db.put(C_POINTS, &point.id, point.to_json());
    s.point_id = point.id.clone();
    engine::session_log(ctx, s, LogLevel::Info, "Restore-Point signiert und veröffentlicht");
    match engine::apply_retention(ctx, job, &repo, &r.id, s) {
        Ok(0) => {}
        Ok(n) => engine::session_log(ctx, s, LogLevel::Info, &format!("Aufbewahrung angewendet: {} alte Restore-Points entfernt", n)),
        Err(e) => {
            s.warnings += 1;
            engine::session_log(ctx, s, LogLevel::Warning, &format!("Aufbewahrung: {}", e));
        }
    }
    engine::refresh_repo_size(ctx, &r.id, &repo);
    let msg = format!("{} Volumes ({}), {} neu gespeichert", snap.roots.len(), size(bytes), size(ws.new_stored));
    if s.errors > 0 { Ok(format!("{} · {} Fehler", msg, s.errors)) } else { Ok(msg) }
}

pub fn start_job(ctx: &Arc<Ctx>, job: &Job, actor: &str) -> Result<String, String> {
    let r: Repository = rec(ctx.get(C_REPOS, &job.repository_id)).ok_or("Das Repository des Jobs fehlt")?;
    let mut session = engine::session_for(ctx, SessionKind::Backup, job, "");
    engine::set_status(ctx, &job.id, SessionStatus::Running, None);
    let job = job.clone();
    let sid = session.id.clone();
    let op = Ops::start(ctx, "job.run", &format!("{}: Sicherung", job.name), actor, &job.id.clone(), &sid, Box::new(move |o| {
        let ctx = o.ctx().clone();
        engine::session_log(&ctx, &session, LogLevel::Info, &format!("Sitzung gestartet: {}", job.name));
        let res = run_backup(o, &ctx, &job, &r, &mut session);
        engine::finish_session(&ctx, &mut session, &res, o.cancelled());
        engine::set_status(&ctx, &job.id, session.status, Some(&session));
        if matches!(session.status, SessionStatus::Success | SessionStatus::Warning) {
            crate::copy::after_backup(&ctx, &job);
            crate::tape::after_backup(&ctx, &job);
        }
        res
    }));
    let _ = ctx.db.update(crate::ctx::C_SESSIONS, &sid, |v| {
        v.insert("operation_id", Value::string(op.as_str()));
    });
    Ok(op)
}

/// the first volume name of a job that cannot be one
pub fn invalid(ids: &[String]) -> Option<String> {
    ids.iter().find(|id| !bc_volume::valid(id)).cloned()
}
