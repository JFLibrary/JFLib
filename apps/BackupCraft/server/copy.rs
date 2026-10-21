// copy -- copy jobs: the restore points of another job into a second repository (a copy
// off site, on other disks: 3-2-1)
//
//   start_copy(ctx, job, actor)    the source job's points that the copy's retention keeps and
//                                  the target repository lacks, oldest first
//   after_backup(ctx, job)         copy jobs without a schedule run after each good backup of
//                                  their source job
//
// Chunk ids are keyed per repository, so a copy rebuilds every tree in the target: file data
// is read from the source repository and put into the target (deduplicated there), trees are
// written anew.  A tree that was copied before maps to its copy directly: the mapping (source
// tree id -> target tree id) of the last run is kept in <data>/copy/<job id>.map, so a copy of
// an incremental point only reads what changed.  The copied point keeps its original time and
// names its source (copied_from); retention and immutability are the copy's own.

use alloc::boxed::Box;
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

use bc_proto::j::{self, Json, Value};
use bc_proto::model::*;
use cas::{CasError, ChunkId, Node, NodeKind, Repo, Snapshot, Tree, KIND_DATA};

use crate::ctx::{Ctx, C_JOBS, C_POINTS, C_REPOS};
use crate::engine::{self, size};
use crate::ops::{OpCtl, Ops};
use crate::util::{all, new_id, now, rec};

const DAY: i64 = 86_400;

fn map_path(ctx: &Ctx, job: &str) -> String {
    plat::fs::join(&ctx.path("copy"), &format!("{}.map", job))
}

fn load_map(ctx: &Ctx, job: &str) -> BTreeMap<ChunkId, ChunkId> {
    let mut m = BTreeMap::new();
    if let Some(b) = plat::read_file(&map_path(ctx, job)) {
        for e in b.chunks_exact(64) {
            let (mut a, mut c) = ([0u8; 32], [0u8; 32]);
            a.copy_from_slice(&e[..32]);
            c.copy_from_slice(&e[32..]);
            m.insert(a, c);
        }
    }
    m
}

fn save_map(ctx: &Ctx, job: &str, m: &BTreeMap<ChunkId, ChunkId>) {
    let mut b = Vec::with_capacity(m.len() * 64);
    for (a, c) in m {
        b.extend_from_slice(a);
        b.extend_from_slice(c);
    }
    let _ = plat::fs::mkdir_private(&ctx.path("copy"));
    let _ = plat::fs::write_file_private(&map_path(ctx, job), &b);
}

/// the points of the source job that the copy wants (kept by its retention, not copied yet)
fn wanted(ctx: &Ctx, job: &Job) -> Vec<RestorePoint> {
    let points: Vec<RestorePoint> = all(&ctx.db, C_POINTS);
    let copied: BTreeSet<String> = points.iter().filter(|p| p.job_id == job.id && p.repository_id == job.repository_id).map(|p| p.copied_from.clone()).collect();
    let src: Vec<RestorePoint> = points.into_iter().filter(|p| p.job_id == job.source_job && p.repository_id != job.repository_id).collect();
    let keep = engine::keep_set(&src, &job.retention, now());
    let mut v: Vec<RestorePoint> = src.into_iter().filter(|p| keep.contains(&p.id) && !copied.contains(&p.id)).collect();
    v.sort_by_key(|p| p.order());
    v
}

pub fn start_copy(ctx: &Arc<Ctx>, job: &Job, actor: &str) -> Result<String, String> {
    if ctx.ops.busy(&job.id, "job.run") {
        return Err(String::from("Der Job läuft bereits"));
    }
    let r: Repository = rec(ctx.get(C_REPOS, &job.repository_id)).ok_or("Das Ziel-Repository des Kopierjobs fehlt")?;
    if !ctx.db.contains(C_JOBS, &job.source_job) && !all::<RestorePoint>(&ctx.db, C_POINTS).iter().any(|p| p.job_id == job.source_job) {
        return Err(String::from("Der Quelljob des Kopierjobs fehlt"));
    }
    let mut session = engine::session_for(ctx, SessionKind::Copy, job, "");
    engine::set_status(ctx, &job.id, SessionStatus::Running, None);
    let job = job.clone();
    let sid = session.id.clone();
    let title = format!("{}: Kopie", job.name);
    let op = Ops::start(ctx, "job.run", &title, actor, &job.id.clone(), &sid, Box::new(move |o| {
        let ctx = o.ctx().clone();
        engine::session_log(&ctx, &session, LogLevel::Info, &format!("Kopie gestartet: {}", job.name));
        let res = copy(o, &ctx, &job, &r, &mut session);
        engine::finish_session(&ctx, &mut session, &res, o.cancelled());
        engine::set_status(&ctx, &job.id, session.status, Some(&session));
        res
    }));
    let _ = ctx.db.update(crate::ctx::C_SESSIONS, &sid, |v| {
        v.insert("operation_id", Value::string(op.as_str()));
    });
    Ok(op)
}

struct Copier<'a> {
    o: &'a OpCtl,
    src: &'a Repo,
    w: cas::Writer<'a>,
    dst: &'a Repo,
    /// source tree -> target tree (this run and the last)
    trees: BTreeMap<ChunkId, ChunkId>,
    /// source chunk -> target chunk (this run)
    chunks: BTreeMap<ChunkId, ChunkId>,
    done: u64,
    total: u64,
    files: u64,
}

impl Copier<'_> {
    fn node(&mut self, n: &Node) -> Result<Node, CasError> {
        if self.o.cancelled() {
            return Err(CasError::Cancelled);
        }
        let mut out = n.clone();
        match n.kind {
            NodeKind::File => {
                self.files += 1;
                let mut ids = Vec::with_capacity(n.chunks.len());
                for id in &n.chunks {
                    if let Some(d) = self.chunks.get(id) {
                        ids.push(*d);
                        continue;
                    }
                    let data = self.src.read_chunk(id)?;
                    self.done += data.len() as u64;
                    let d = self.w.put(KIND_DATA, &data)?;
                    if self.chunks.len() < 2_000_000 {
                        self.chunks.insert(*id, d);
                    }
                    ids.push(d);
                }
                out.chunks = ids;
                self.o.progress(self.done, self.total.max(self.done));
            }
            NodeKind::Dir => {
                if let Some(t) = n.tree {
                    match self.trees.get(&t) {
                        Some(d) if self.dst.has_chunk(d) => {
                            out.tree = Some(*d);
                            self.done += n.size;
                        }
                        _ => {
                            let tree = self.src.read_tree(&t)?;
                            let mut nt = Tree::default();
                            for c in &tree.nodes {
                                nt.nodes.push(self.node(c)?);
                            }
                            let d = self.w.put_tree(&nt)?;
                            self.trees.insert(t, d);
                            out.tree = Some(d);
                        }
                    }
                }
            }
            _ => {}
        }
        Ok(out)
    }
}

fn copy(o: &OpCtl, ctx: &Arc<Ctx>, job: &Job, r: &Repository, s: &mut Session) -> Result<String, String> {
    let dst = engine::open_repo(ctx, r)?;
    engine::ensure_job_key(ctx, job, &dst, s)?;
    let todo = wanted(ctx, job);
    if todo.is_empty() {
        engine::session_log(ctx, s, LogLevel::Info, "Alle Restore-Points des Quelljobs sind schon kopiert");
    }
    let mut trees = load_map(ctx, &job.id);
    let mut copied = 0;
    let mut bytes = 0u64;
    let mut new_stored = 0u64;
    let (codec, level) = engine::codec_of(job.compression);
    let total: u64 = todo.iter().map(|p| p.size).sum();
    s.bytes_total = total;
    for p in &todo {
        if o.cancelled() {
            return Err(String::from("Abgebrochen"));
        }
        let (_, src) = engine::repo_for(ctx, &p.repository_id)?;
        let snap = src.load_snapshot(&p.id).map_err(engine::err_text)?;
        engine::session_log(ctx, s, LogLevel::Info, &format!("Restore-Point vom {} wird kopiert ({})", crate::notify::when(p.created), size(p.size)));
        let mut c = Copier { o, src: &src, w: dst.writer().codec(codec, level), dst: &dst, trees: core::mem::take(&mut trees), chunks: BTreeMap::new(), done: 0, total: p.size, files: 0 };
        let mut roots = Vec::new();
        for n in &snap.roots {
            match c.node(n) {
                Ok(x) => roots.push(x),
                Err(CasError::Cancelled) => return Err(String::from("Abgebrochen")),
                Err(e) => return Err(engine::err_text(e)),
            }
        }
        trees = core::mem::take(&mut c.trees);
        let files = c.files;
        let ws = c.w.finish().map_err(engine::err_text)?;
        let imm = dst.immutable_days();
        let t = now();
        let parent = all::<RestorePoint>(&ctx.db, C_POINTS).into_iter().filter(|x| x.job_id == job.id && x.repository_id == r.id).max_by_key(|x| x.order()).map_or(String::new(), |x| x.id);
        let mut stats = snap.stats;
        stats.new_bytes = ws.new_stored;
        stats.new_chunks = ws.new_chunks;
        stats.new_raw = ws.new_raw;
        let copy = Snapshot {
            id: new_id(),
            created: p.created,
            job_id: job.id.clone(),
            job_name: job.name.clone(),
            host: snap.host.clone(),
            roots,
            parent,
            full: snap.full,
            stats,
            immutable_until: if imm > 0 { t + imm as i64 * DAY } else { 0 },
            meta: j::obj([
                ("created_ns", Value::i64(p.order())),
                ("copied_from", Value::string(p.id.as_str())),
                ("source_job", Value::string(p.job_id.as_str())),
                ("source_repository", Value::string(p.repository_id.as_str())),
                ("session", Value::string(s.id.as_str())),
            ]),
        };
        dst.save_snapshot(&copy, &ctx.keys.signing).map_err(engine::err_text)?;
        let point = engine::point_record(&copy, &r.id, job.encrypted);
        let _ = ctx.db.put(C_POINTS, &point.id, point.to_json());
        s.point_id = point.id.clone();
        copied += 1;
        bytes += p.size;
        new_stored += ws.new_stored;
        s.files_total += files;
        s.files_done = s.files_total;
        s.bytes_read = bytes;
        s.bytes_new = new_stored;
        engine::save(ctx, s);
        engine::session_log(ctx, s, LogLevel::Info, &format!("{} Dateien, {} neu im Ziel", files, size(ws.new_stored)));
    }
    // only the trees of the newest copy are worth keeping for the next run
    if let Some(last) = all::<RestorePoint>(&ctx.db, C_POINTS).into_iter().filter(|x| x.job_id == job.id && x.repository_id == r.id).max_by_key(|x| x.order()) {
        let mut live = BTreeSet::new();
        if let Ok(snap) = dst.load_snapshot(&last.id) {
            let _ = dst.referenced(&snap, &mut live);
        }
        trees.retain(|_, d| live.contains(d));
        save_map(ctx, &job.id, &trees);
    }
    match engine::apply_retention(ctx, job, &dst, &r.id, s) {
        Ok(0) => {}
        Ok(n) => engine::session_log(ctx, s, LogLevel::Info, &format!("Aufbewahrung angewendet: {} alte Kopien entfernt", n)),
        Err(e) => {
            s.warnings += 1;
            engine::session_log(ctx, s, LogLevel::Warning, &format!("Aufbewahrung: {}", e));
        }
    }
    engine::refresh_repo_size(ctx, &r.id, &dst);
    Ok(format!("{} Restore-Points kopiert ({}), {} neu gespeichert", copied, size(bytes), size(new_stored)))
}

/// copy jobs without a schedule follow their source job
pub fn after_backup(ctx: &Ctx, source: &Job) {
    let Some(arc) = crate::ctx::current() else { return };
    for j in all::<Job>(&ctx.db, C_JOBS) {
        if j.kind == JobKind::Copy && j.enabled && j.source_job == source.id && j.schedule.cron.trim().is_empty() {
            let _ = start_copy(&arc, &j, "Nach Quelljob");
        }
    }
}
