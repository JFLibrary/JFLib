// engine -- what the API asks of the backup engine, on top of lib/cas
//
//   repositories   create / attach (its restore points become known), immutability, check,
//                  clean up (gc); opened once and kept (one process holds a repository)
//   backup         walk the sources of the job's host (the server itself; agents come with
//                  TLS), reuse the chunks of unchanged files (same size, modification time and
//                  file id as in the previous point; "active full" reads everything), store
//                  trees and a signed snapshot, record the restore point, apply the retention
//   restore        selected entries to the original place or a folder; never through a
//                  symbolic link in the target, never into a folder that is not allowed,
//                  existing files skipped, renamed or (operators only) replaced; files are
//                  written next to their target and renamed when complete
//   verify         read every chunk of a point (decrypt, compare with its id)
//   browse         the tree of a point, page by page
//
// The repository key is wrapped for a key derived from the server's master key; the recovery
// kit's public key (if there is one) gets a sealed copy; an encrypted job's passphrase opens
// the repository too (restores without this server).

use alloc::boxed::Box;
use alloc::collections::BTreeSet;
use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

use bc_proto::j::{Json, Value};
use bc_proto::model::*;
use cas::{CasError, CreateOptions, Node, NodeKind, Repo, Snapshot, Tree, Unlock};

use crate::ctx::{Ctx, C_HOSTS, C_JOBS, C_POINTS, C_REPOS, C_SESSIONS};
use crate::ops::{OpCtl, Ops};
use crate::util::{all, new_id, now, rec};

const DAY: i64 = 86_400;

pub struct RepoInfo {
    pub id: String,
    pub used: u64,
    pub immutable_days: u32,
}

fn cas_err(e: CasError) -> String {
    format!("{}", e)
}

fn wrap_key(ctx: &Ctx) -> [u8; 32] {
    ctx.keys.derive("BackupCraft 2026 repository wrap")
}

/// the open repository of a record
pub fn open_repo(ctx: &Ctx, r: &Repository) -> Result<Arc<Repo>, String> {
    if let Some(x) = ctx.repos.lock().get(&r.id) {
        return Ok(x.clone());
    }
    let k = wrap_key(ctx);
    let repo = Repo::open(&r.path, Unlock::Master(&k)).map_err(|e| match e {
        CasError::Locked => String::from("Das Repository gehört zu einem anderen Server (zum Einbinden das Recovery-Kit verwenden)"),
        e => format!("Repository „{}“: {}", r.name, e),
    })?;
    let repo = Arc::new(repo);
    ctx.repos.lock().insert(r.id.clone(), repo.clone());
    Ok(repo)
}

fn repo_of(ctx: &Ctx, id: &str) -> Result<(Repository, Arc<Repo>), String> {
    let r: Repository = rec(ctx.get(C_REPOS, id)).ok_or("Repository nicht gefunden")?;
    let repo = open_repo(ctx, &r)?;
    Ok((r, repo))
}

pub fn repo_init(ctx: &Ctx, path: &str, id: &str) -> Result<RepoInfo, String> {
    let o = CreateOptions {
        id: String::from(id),
        chunk_avg: 1 << 20,
        compression: compress::Codec::Lz4,
        master: wrap_key(ctx),
        recovery: ctx.keys.recovery_public(),
        signer: ctx.keys.signing.public,
        immutable_days: 0,
    };
    let repo = Repo::create(path, &o).map_err(|e| match e {
        CasError::NotEmpty => String::from("Der Ordner ist nicht leer (zum Einbinden „vorhandenes Repository“ wählen)"),
        e => cas_err(e),
    })?;
    ctx.repos.lock().insert(String::from(id), Arc::new(repo));
    Ok(RepoInfo { id: String::from(id), used: 0, immutable_days: 0 })
}

pub fn repo_attach(ctx: &Ctx, path: &str) -> Result<RepoInfo, String> {
    let id = cas::peek_id(path).map_err(|_| String::from("Kein BackupCraft-Repository in diesem Ordner"))?;
    let k = wrap_key(ctx);
    let repo = Repo::open(path, Unlock::Master(&k)).map_err(|e| match e {
        CasError::Locked => String::from("Das Repository gehört zu einem anderen Server (zum Einbinden das Recovery-Kit verwenden)"),
        e => cas_err(e),
    })?;
    // the restore points it holds become known (jobs may be gone: the points stay restorable)
    let mut known: Vec<String> = Vec::new();
    for sid in repo.snapshot_ids() {
        if ctx.db.contains(C_POINTS, &sid) {
            continue;
        }
        if let Ok(s) = repo.load_snapshot(&sid) {
            let p = point_of(&s, &id, false);
            let _ = ctx.db.put(C_POINTS, &p.id, p.to_json());
            known.push(sid);
        }
    }
    let info = RepoInfo { id: id.clone(), used: repo.stored_bytes(), immutable_days: repo.immutable_days() };
    ctx.repos.lock().insert(id, Arc::new(repo));
    Ok(info)
}

/// seal a repository's key for the recovery kit
pub fn seal_for_recovery(ctx: &Ctx, r: &Repository, pk: &[u8; 32]) -> Result<(), String> {
    let k = wrap_key(ctx);
    open_repo(ctx, r)?.set_recovery(pk, Unlock::Master(&k)).map_err(cas_err)
}

pub fn repo_set_immutable(ctx: &Ctx, r: &Repository, days: u32) -> Result<(), String> {
    open_repo(ctx, r)?.set_immutable_days(days).map_err(cas_err)
}

pub fn repo_check(o: &OpCtl, r: &Repository, full: bool) -> Result<String, String> {
    let repo = open_repo(o.ctx(), r)?;
    let rep = repo.check(full, &mut |d, t| {
        o.progress(d, t);
        !o.cancelled()
    }).map_err(cas_err)?;
    if rep.ok() {
        let mut s = format!("{} Restore-Points, {} Chunks in Ordnung", rep.snapshots, rep.chunks_referenced);
        if rep.orphan_packs > 0 {
            s.push_str(&format!(" · {} unvollständige Pakete (werden beim Aufräumen entfernt)", rep.orphan_packs));
        }
        Ok(s)
    } else {
        Err(format!("{} Restore-Points beschädigt, {} Chunks fehlen, {} Chunks beschädigt", rep.bad_snapshots.len(), rep.missing, rep.damaged))
    }
}

pub fn repo_gc(o: &OpCtl, r: &Repository) -> Result<String, String> {
    let repo = open_repo(o.ctx(), r)?;
    let g = repo.gc(&mut |d, t| {
        o.progress(d, t);
        !o.cancelled()
    }).map_err(cas_err)?;
    update_repo_size(o.ctx(), &r.id, &repo);
    Ok(format!("{} freigegeben ({} Pakete gelöscht, {} neu geschrieben)", size(g.bytes_freed), g.packs_deleted, g.packs_repacked))
}

fn update_repo_size(ctx: &Ctx, id: &str, repo: &Repo) {
    let used = repo.stored_bytes();
    let _ = ctx.db.update(C_REPOS, id, |v| {
        v.insert("used", Value::u64(used));
    });
}

/// "1,2 GB" (bc_proto::size_text)
pub fn size(b: u64) -> String {
    bc_proto::size_text(b)
}

/// the restore point record of a snapshot
fn point_of(s: &Snapshot, repo_id: &str, encrypted: bool) -> RestorePoint {
    RestorePoint {
        id: s.id.clone(),
        job_id: s.job_id.clone(),
        job_name: s.job_name.clone(),
        created: s.created,
        full: s.full,
        files: s.stats.files,
        size: s.stats.bytes,
        stored_new: s.stats.new_bytes,
        immutable_until: s.immutable_until,
        verified: 0,
        host: s.host.clone(),
        encrypted,
        signed: true,
        repository_id: String::from(repo_id),
        copied_from: String::from(bc_proto::j::str_of(&s.meta, "copied_from")),
        created_ns: s.meta.get("created_ns").and_then(|v| v.as_i64()).unwrap_or(0),
    }
}

/// the point record of a snapshot (copy jobs)
pub(crate) fn point_record(s: &Snapshot, repo_id: &str, encrypted: bool) -> RestorePoint {
    point_of(s, repo_id, encrypted)
}

pub(crate) fn set_status(ctx: &Ctx, id: &str, st: SessionStatus, s: Option<&Session>) {
    set_job_status(ctx, id, st, s)
}

pub(crate) fn refresh_repo_size(ctx: &Ctx, id: &str, repo: &Repo) {
    update_repo_size(ctx, id, repo)
}

/// let the job passphrase open the repository too (a restore without this server)
pub(crate) fn ensure_job_key(ctx: &Ctx, job: &Job, repo: &Repo, s: &Session) -> Result<(), String> {
    if job.encrypted && !repo.has_passphrase(&format!("job:{}", job.id)) {
        if let Some(pass) = crate::jobs::passphrase(ctx, &job.id) {
            let k = wrap_key(ctx);
            repo.add_passphrase(&pass, &format!("job:{}", job.id), Unlock::Master(&k)).map_err(cas_err)?;
            log(ctx, s, LogLevel::Info, "Passphrase des Jobs im Repository hinterlegt");
        }
    }
    Ok(())
}

/// the retention of a job in one repository, then a clean-up when points went
pub(crate) fn apply_retention(ctx: &Ctx, job: &Job, repo: &Repo, repo_id: &str, s: &Session) -> Result<usize, String> {
    let n = retention(ctx, job, repo, repo_id, s)?;
    if n > 0 {
        if let Ok(g) = repo.gc(&mut |_, _| true) {
            log(ctx, s, LogLevel::Info, &format!("{} freigegeben", size(g.bytes_freed)));
        }
    }
    Ok(n)
}

// ------------------------------------------------------------------ sessions

fn new_session(ctx: &Ctx, kind: SessionKind, job: &Job, point: &str) -> Session {
    let s = Session { id: new_id(), kind, job_id: job.id.clone(), job_name: job.name.clone(), status: SessionStatus::Running, started: now(), point_id: String::from(point), ..Session::default() };
    let _ = ctx.db.put(C_SESSIONS, &s.id, s.to_json());
    s
}

fn save_session(ctx: &Ctx, s: &Session) {
    let _ = ctx.db.put(C_SESSIONS, &s.id, s.to_json());
}

fn log(ctx: &Ctx, s: &Session, level: LogLevel, msg: &str) {
    ctx.log(&s.id, level, msg);
}

/// finish a session from a result
fn close_session(ctx: &Ctx, s: &mut Session, r: &Result<String, String>, cancelled: bool) {
    s.finished = now();
    s.status = match r {
        _ if cancelled => SessionStatus::Cancelled,
        Ok(_) if s.errors > 0 || s.warnings > 0 => SessionStatus::Warning,
        Ok(_) => SessionStatus::Success,
        Err(_) => SessionStatus::Failed,
    };
    match r {
        Ok(m) => s.message = m.clone(),
        Err(e) => s.message = e.clone(),
    }
    let level = match s.status {
        SessionStatus::Failed => LogLevel::Error,
        SessionStatus::Warning | SessionStatus::Cancelled => LogLevel::Warning,
        _ => LogLevel::Info,
    };
    log(ctx, s, level, &format!("Sitzung beendet: {}", s.message));
    save_session(ctx, s);
    crate::notify::session_finished(ctx, s);
}

// ------------------------------------------------------------------ backup

/// start a backup of a job -> the operation id
pub fn start_backup(ctx: &Arc<Ctx>, job: &Job, full: bool, actor: &str) -> Result<String, String> {
    start_backup_hint(ctx, job, full, actor, None)
}

/// a backup that knows the changed paths (CDP; None: walk everything); a CDP run (actor "CDP")
/// makes no point when nothing changed
pub fn start_backup_hint(ctx: &Arc<Ctx>, job: &Job, full: bool, actor: &str, hint: Option<Vec<String>>) -> Result<String, String> {
    let cdp = actor == "CDP";
    if ctx.ops.busy(&job.id, "job.run") {
        return Err(String::from("Der Job läuft bereits"));
    }
    if job.kind == JobKind::Copy {
        return crate::copy::start_copy(ctx, job, actor);
    }
    if job.kind == JobKind::Tape {
        return crate::tape::start_job(ctx, job, actor);
    }
    if job.kind == JobKind::Qemu {
        return crate::vm::start_job(ctx, job, full, actor);
    }
    if job.kind == JobKind::Volume {
        return crate::volume::start_job(ctx, job, actor);
    }
    if job.kind != JobKind::File {
        return Err(String::from("Diese Art von Job wird hier nicht gesichert"));
    }
    let host: Host = rec(ctx.get(C_HOSTS, &job.host_id)).ok_or("Der Host des Jobs fehlt")?;
    let r: Repository = rec(ctx.get(C_REPOS, &job.repository_id)).ok_or("Das Repository des Jobs fehlt")?;
    let mut session = new_session(ctx, SessionKind::Backup, job, "");
    set_job_status(ctx, &job.id, SessionStatus::Running, None);
    let job = job.clone();
    let title = format!("{}: {}", job.name, if full { "Aktive Vollsicherung" } else { "Sicherung" });
    let sid = session.id.clone();
    let op = Ops::start(ctx, "job.run", &title, actor, &job.id.clone(), &sid, Box::new(move |o| {
        let ctx = o.ctx().clone();
        log(&ctx, &session, LogLevel::Info, &format!("Sitzung gestartet: {}", job.name));
        let r = backup(o, &ctx, &job, &host, &r, full, &mut session, &host.roots, true, &[], hint, cdp);
        close_session(&ctx, &mut session, &r, o.cancelled());
        set_job_status(&ctx, &job.id, session.status, Some(&session));
        if matches!(session.status, SessionStatus::Success | SessionStatus::Warning) {
            crate::copy::after_backup(&ctx, &job);
            crate::replica::after_backup(&ctx, &job);
            crate::tape::after_backup(&ctx, &job);
        }
        r
    }));
    let _ = ctx.db.update(C_SESSIONS, &sid, |v| {
        v.insert("operation_id", Value::string(op.as_str()));
    });
    Ok(op)
}

fn set_job_status(ctx: &Ctx, id: &str, st: SessionStatus, s: Option<&Session>) {
    let points = all::<RestorePoint>(&ctx.db, C_POINTS).iter().filter(|p| p.job_id == id).count() as u32;
    let _ = ctx.db.update(C_JOBS, id, |v| {
        v.insert("last_status", bc_proto::j::Field::put(&st));
        v.insert("points", Value::u64(points as u64));
        if let Some(s) = s {
            v.insert("last_run", Value::i64(s.started));
            if s.bytes_read > 0 || s.status == SessionStatus::Success {
                v.insert("last_size", Value::u64(s.bytes_total));
            }
        }
    });
}

/// whether a name or path matches an exclude pattern (* and ?; a pattern with a separator
/// compares the whole path)
pub fn excluded(patterns: &[String], name: &str, path: &str) -> bool {
    patterns.iter().any(|p| {
        let p = p.trim();
        if p.is_empty() {
            return false;
        }
        let subject = if p.contains(['/', '\\']) { path } else { name };
        let (a, b) = if cfg!(windows) { (p.to_lowercase(), subject.to_lowercase()) } else { (String::from(p), String::from(subject)) };
        glob(a.as_bytes(), b.as_bytes())
    })
}

fn glob(p: &[u8], s: &[u8]) -> bool {
    let (mut pi, mut si, mut star, mut mark) = (0, 0, None, 0);
    while si < s.len() {
        let sep = |c: u8| c == b'/' || c == b'\\';
        if pi < p.len() && (p[pi] == b'?' || p[pi] == s[si] || (sep(p[pi]) && sep(s[si]))) {
            pi += 1;
            si += 1;
        } else if pi < p.len() && p[pi] == b'*' {
            star = Some(pi);
            mark = si;
            pi += 1;
        } else if let Some(st) = star {
            pi = st + 1;
            mark += 1;
            si = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == b'*' {
        pi += 1;
    }
    pi == p.len()
}

/// where a backup reads from: this machine or an agent
pub(crate) enum Src<'a> {
    Local,
    Agent(&'a Host),
}

/// an entry's metadata, whichever source it came from
#[derive(Clone, Default)]
pub(crate) struct M {
    pub kind: NodeKind,
    pub size: u64,
    pub mtime_ns: i64,
    pub atime_ns: i64,
    pub ctime_ns: i64,
    pub mode: u32,
    pub uid: u32,
    pub gid: u32,
    pub attrs: u32,
    pub ino: u64,
    pub dev: u64,
    pub link: String,
}

fn m_local(path: &str) -> Option<M> {
    bc_fs::entry(path, "").map(|e| m_agent(&e))
}

fn m_agent(e: &bc_proto::agent::AgentEntry) -> M {
    let kind = match e.kind.as_str() {
        "dir" => NodeKind::Dir,
        "file" => NodeKind::File,
        "link" => NodeKind::Symlink,
        _ => NodeKind::Other,
    };
    M { kind, size: e.size, mtime_ns: e.mtime_ns, atime_ns: e.atime_ns, ctime_ns: e.ctime_ns, mode: e.mode, uid: e.uid, gid: e.gid, attrs: e.attrs, ino: e.ino, dev: e.dev, link: e.link.clone() }
}

impl Src<'_> {
    fn join(&self, dir: &str, name: &str) -> String {
        match self {
            Src::Local => plat::fs::join(dir, name),
            Src::Agent(h) => {
                let sep = if h.os == Os::Windows { '\\' } else { '/' };
                let mut s = String::from(dir);
                if !s.ends_with(['/', '\\']) {
                    s.push(sep);
                }
                s.push_str(name);
                s
            }
        }
    }

    fn stat(&self, ctx: &Ctx, path: &str) -> Result<M, String> {
        match self {
            Src::Local => m_local(path).ok_or_else(|| String::from("nicht lesbar")),
            Src::Agent(h) => crate::agents::stat(ctx, h, path).map(|e| m_agent(&e)),
        }
    }

    /// the entries of a directory, sorted by name
    fn list(&self, ctx: &Ctx, path: &str) -> Result<Vec<(String, M)>, String> {
        let mut out: Vec<(String, M)> = match self {
            Src::Local => {
                let l = plat::fs::read_dir(path).ok_or_else(|| String::from("Ordner nicht lesbar"))?;
                l.into_iter().map(|e| {
                    let m = m_local(&plat::fs::join(path, &e.name)).unwrap_or(M { kind: NodeKind::Other, ..M::default() });
                    (e.name, m)
                }).collect()
            }
            Src::Agent(h) => {
                let mut v = Vec::new();
                let mut off = 0;
                loop {
                    let l = crate::agents::list(ctx, h, path, off)?;
                    let n = l.entries.len();
                    v.extend(l.entries.iter().map(|e| (e.name.clone(), m_agent(e))));
                    off += n;
                    if !l.more || n == 0 {
                        break;
                    }
                }
                v
            }
        };
        out.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(out)
    }
}

/// the changed paths a CDP run knows of: folders outside them are taken over from the previous
/// point without listing them
pub(crate) struct Hint {
    changed: BTreeSet<String>,
    /// the changed paths and every folder above them
    touched: BTreeSet<String>,
    win: bool,
}

impl Hint {
    pub(crate) fn new(paths: &[String], win: bool) -> Hint {
        let norm = |p: &str| -> String {
            let t = p.trim_end_matches(['/', '\\']);
            if win { t.replace('/', "\\").to_lowercase() } else { String::from(t) }
        };
        let mut h = Hint { changed: BTreeSet::new(), touched: BTreeSet::new(), win };
        for p in paths {
            let c = norm(p);
            let mut a = c.clone();
            loop {
                h.touched.insert(a.clone());
                match a.rfind(['/', '\\']) {
                    Some(i) if i > 0 => a.truncate(i),
                    _ => break,
                }
            }
            h.changed.insert(c);
        }
        h
    }

    fn norm(&self, p: &str) -> String {
        let t = p.trim_end_matches(['/', '\\']);
        if self.win { t.replace('/', "\\").to_lowercase() } else { String::from(t) }
    }

    /// whether anything at or below `path` may have changed
    fn touched(&self, path: &str) -> bool {
        let mut a = self.norm(path);
        if self.touched.contains(&a) {
            return true;
        }
        // below a changed folder (a folder that moved in, a lost subtree)
        loop {
            if self.changed.contains(&a) {
                return true;
            }
            match a.rfind(['/', '\\']) {
                Some(i) if i > 0 => a.truncate(i),
                _ => return false,
            }
        }
    }
}

struct Walk<'a> {
    o: &'a OpCtl,
    ctx: &'a Ctx,
    repo: &'a Repo,
    src: Src<'a>,
    w: cas::Writer<'a>,
    full: bool,
    excludes: &'a [String],
    session: &'a mut Session,
    stats: cas::SnapStats,
    /// bytes read or taken over so far / the expected total
    done: u64,
    total: u64,
    last_save: i64,
    hint: Option<Hint>,
    /// something differs from the previous point: a file read, an entry added or gone, a link
    changed: bool,
}

impl Walk<'_> {
    /// files and bytes below a directory node (older points: counted from their trees)
    fn totals(&self, n: &Node) -> Result<(u64, u64), CasError> {
        if n.count > 0 || n.size > 0 || n.tree.is_none() {
            return Ok((n.count, n.size));
        }
        let tree = self.repo.read_tree(&n.tree.unwrap_or([0; 32]))?;
        let mut out = (0, 0);
        for c in &tree.nodes {
            match c.kind {
                NodeKind::File => {
                    out.0 += 1;
                    out.1 += c.size;
                }
                NodeKind::Dir => {
                    let (f, b) = self.totals(c)?;
                    out.0 += f;
                    out.1 += b;
                }
                _ => {}
            }
        }
        Ok(out)
    }

    fn progress(&mut self) {
        self.o.progress(self.done, self.total.max(self.done));
        let t = now();
        if t - self.last_save >= 2 {
            self.last_save = t;
            self.session.files_done = self.stats.files;
            self.session.bytes_read = self.done;
            self.session.bytes_total = self.total.max(self.done);
            save_session(self.ctx, self.session);
        }
    }

    fn warn(&mut self, path: &str, what: &str) -> String {
        self.session.warnings += 1;
        self.stats.errors += 1;
        let m = format!("{}: {}", path, what);
        log(self.ctx, self.session, LogLevel::Warning, &m);
        String::from(what)
    }

    /// the node of a path with its metadata (recursing into directories); `prev`: the same
    /// entry in the previous point
    fn node(&mut self, path: &str, name: &str, m: M, prev: Option<&Node>) -> Result<Option<Node>, CasError> {
        if self.o.cancelled() {
            return Err(CasError::Cancelled);
        }
        let mut n = Node {
            name: String::from(name),
            kind: m.kind,
            size: m.size,
            mtime_ns: m.mtime_ns,
            atime_ns: m.atime_ns,
            ctime_ns: m.ctime_ns,
            mode: m.mode,
            uid: m.uid,
            gid: m.gid,
            attrs: m.attrs,
            ino: m.ino,
            dev: m.dev,
            ..Node::default()
        };
        match m.kind {
            NodeKind::Dir => {
                // CDP: a folder nothing below changed is the previous point's
                if let (Some(h), Some(p)) = (&self.hint, prev) {
                    if !self.full && p.kind == NodeKind::Dir && p.error.is_empty() && p.tree.is_some_and(|t| self.repo.has_chunk(&t)) && !h.touched(path) {
                        let (files, bytes) = self.totals(p)?;
                        let mut keep = p.clone();
                        keep.name = String::from(name);
                        keep.count = files;
                        keep.size = bytes;
                        self.stats.dirs += 1;
                        self.stats.files += files;
                        self.stats.bytes += bytes;
                        self.done += bytes;
                        self.progress();
                        return Ok(Some(keep));
                    }
                }
                n.size = 0;
                self.stats.dirs += 1;
                let prev_tree = match prev.and_then(|p| p.tree) {
                    Some(t) => self.repo.read_tree(&t).ok(),
                    None => None,
                };
                let list = match self.src.list(self.ctx, path) {
                    Ok(l) => l,
                    Err(e) => {
                        n.error = self.warn(path, &e);
                        self.changed = true;
                        return Ok(Some(n));
                    }
                };
                // entries added or gone (excluded ones do not count)
                match &prev_tree {
                    Some(t) => {
                        let now: BTreeSet<&str> = list.iter().map(|(c, _)| c.as_str()).filter(|c| !excluded(self.excludes, c, &self.src.join(path, c))).collect();
                        let before: BTreeSet<&str> = t.nodes.iter().map(|c| c.name.as_str()).collect();
                        if now != before {
                            self.changed = true;
                        }
                    }
                    None => self.changed = true,
                }
                let mut tree = Tree::default();
                for (cname, cm) in list {
                    let child = self.src.join(path, &cname);
                    if excluded(self.excludes, &cname, &child) {
                        continue;
                    }
                    let p = prev_tree.as_ref().and_then(|t| t.find(&cname));
                    if let Some(c) = self.node(&child, &cname, cm, p)? {
                        tree.nodes.push(c);
                    }
                }
                // the folder's totals (CDP takes unchanged folders over with them)
                for c in &tree.nodes {
                    match c.kind {
                        NodeKind::File => {
                            n.count += 1;
                            n.size += c.size;
                        }
                        NodeKind::Dir => {
                            n.count += c.count;
                            n.size += c.size;
                        }
                        _ => {}
                    }
                }
                n.tree = Some(self.w.put_tree(&tree)?);
            }
            NodeKind::Symlink => {
                n.link = m.link.clone();
                if prev.is_none_or(|p| p.link != n.link) {
                    self.changed = true;
                }
            }
            NodeKind::File => {
                self.stats.files += 1;
                self.stats.bytes += m.size;
                // unchanged: take the chunks over without reading
                if let Some(p) = prev.filter(|p| !self.full && p.kind == NodeKind::File && p.error.is_empty()) {
                    if p.size == m.size && p.mtime_ns == m.mtime_ns && (p.ino == m.ino || p.ino == 0 || m.ino == 0) && self.repo_has(&p.chunks) {
                        n.chunks = p.chunks.clone();
                        self.done += m.size;
                        self.progress();
                        return Ok(Some(n));
                    }
                }
                self.changed = true;
                let r = match &self.src {
                    Src::Local => match plat::File::open(&plat::fs::long_path(path), plat::FO_READ) {
                        None => Err(CasError::Io(String::from("Datei nicht lesbar (gesperrt oder keine Berechtigung)"))),
                        Some(f) => {
                            let mut off = 0u64;
                            let mut read = |buf: &mut [u8]| -> Result<usize, String> {
                                let k = f.read_at(off, buf).ok_or_else(|| format!("Lesefehler in {}", path))?;
                                off += k as u64;
                                Ok(k)
                            };
                            let o = self.o;
                            self.w.put_stream(&mut read, &|| o.cancelled())
                        }
                    },
                    Src::Agent(h) => {
                        let (ctx, h) = (self.ctx, *h);
                        let mut off = 0u64;
                        let mut pending: Vec<u8> = Vec::new();
                        let mut read = |buf: &mut [u8]| -> Result<usize, String> {
                            if pending.is_empty() {
                                pending = crate::agents::read(ctx, h, path, off, bc_proto::agent::READ_MAX)?;
                                off += pending.len() as u64;
                            }
                            let k = buf.len().min(pending.len());
                            buf[..k].copy_from_slice(&pending[..k]);
                            pending.drain(..k);
                            Ok(k)
                        };
                        let o = self.o;
                        self.w.put_stream(&mut read, &|| o.cancelled())
                    }
                };
                match r {
                    Ok((ids, got)) => {
                        n.chunks = ids;
                        if got != m.size {
                            n.size = got;
                            self.warn(path, "hat sich während der Sicherung geändert");
                        }
                        self.done += got;
                    }
                    Err(CasError::Cancelled) => return Err(CasError::Cancelled),
                    Err(CasError::Io(e)) => n.error = self.warn(path, &e),
                    Err(e) => return Err(e),
                }
                self.progress();
            }
            NodeKind::Other => {}
        }
        Ok(Some(n))
    }

    fn repo_has(&self, chunks: &[cas::ChunkId]) -> bool {
        chunks.iter().all(|c| self.repo.has_chunk(c))
    }
}

/// the codec and zstd level of a job's compression setting
pub fn codec_of(c: Compression) -> (compress::Codec, i32) {
    match c {
        Compression::None => (compress::Codec::None, 0),
        Compression::Lz4 => (compress::Codec::Lz4, 0),
        Compression::Zstd => (compress::Codec::Zstd, compress::zstd::LEVEL_DEFAULT),
        Compression::ZstdHigh => (compress::Codec::Zstd, 6),
    }
}

/// the newest restore point of a job (for the incremental comparison)
pub(crate) fn previous(ctx: &Ctx, job: &Job, repo_id: &str) -> Option<RestorePoint> {
    all::<RestorePoint>(&ctx.db, C_POINTS).into_iter().filter(|p| p.job_id == job.id && p.repository_id == repo_id).max_by_key(|p| p.order())
}

/// back up folders of a host (inside `allowed`) as a point of the job, without retention, each
/// stored under the name the job knows it by (job.sources[i] read, names[i] stored) -> the
/// new point's id (replica failback)
pub(crate) fn backup_folders(o: &OpCtl, ctx: &Arc<Ctx>, job: &Job, host: &Host, r: &Repository, allowed: &[String], names: &[String], s: &mut Session) -> Result<String, String> {
    backup(o, ctx, job, host, r, true, s, allowed, false, names, None, false)?;
    Ok(s.point_id.clone())
}

#[allow(clippy::too_many_arguments)]
fn backup(o: &OpCtl, ctx: &Arc<Ctx>, job: &Job, host: &Host, r: &Repository, full: bool, s: &mut Session, allowed: &[String], keep_rules: bool, names: &[String],
          hint: Option<Vec<String>>, skip_unchanged: bool) -> Result<String, String> {
    let repo = open_repo(ctx, r)?;
    // the job passphrase opens the repository too (a restore without this server)
    if job.encrypted && !repo.has_passphrase(&format!("job:{}", job.id)) {
        if let Some(pass) = crate::jobs::passphrase(ctx, &job.id) {
            let k = wrap_key(ctx);
            repo.add_passphrase(&pass, &format!("job:{}", job.id), Unlock::Master(&k)).map_err(cas_err)?;
            log(ctx, s, LogLevel::Info, "Passphrase des Jobs im Repository hinterlegt");
        }
    }
    let prev = previous(ctx, job, &r.id);
    let prev_snap = prev.as_ref().and_then(|p| repo.load_snapshot(&p.id).ok());
    let due_full = job.active_full_days > 0 && prev.as_ref().is_some_and(|p| {
        // the last point that read everything
        let last_full = all::<RestorePoint>(&ctx.db, C_POINTS).into_iter().filter(|x| x.job_id == job.id && x.full).map(|x| x.created).max().unwrap_or(0);
        now() - last_full >= job.active_full_days as i64 * DAY && p.full == false
    });
    let full = full || due_full || prev_snap.is_none();
    if full {
        log(ctx, s, LogLevel::Info, if prev_snap.is_none() { "Erste Sicherung: alle Dateien werden gelesen" } else { "Aktive Vollsicherung: alle Dateien werden gelesen" });
    }
    let total = prev.as_ref().map_or(0, |p| p.size);
    let src = if host.kind == HostKind::Agent { Src::Agent(host) } else { Src::Local };
    if let Src::Agent(h) = &src {
        crate::agents::info(ctx, h).map_err(|e| format!("Agent nicht erreichbar: {}", e))?;
    }
    let (codec, level) = codec_of(job.compression);
    let hint = hint.filter(|_| !full && prev_snap.is_some()).map(|h| {
        log(ctx, s, LogLevel::Info, &format!("CDP: {} geänderte Pfade, unveränderte Ordner werden übernommen", h.len()));
        Hint::new(&h, crate::hosts::host_windows(host))
    });
    let mut walk = Walk { o, ctx, repo: &repo, src, w: repo.writer().codec(codec, level), full, excludes: &job.excludes, session: s, stats: cas::SnapStats::default(), done: 0, total, last_save: 0, hint, changed: false };
    let mut roots = Vec::new();
    for (k, src) in job.sources.iter().enumerate() {
        let stored = names.get(k).unwrap_or(src);
        if !crate::hosts::inside(allowed, src) {
            walk.warn(src, "liegt nicht (mehr) in einem freigegebenen Ordner");
            continue;
        }
        log(ctx, walk.session, LogLevel::Info, &format!("Quelle wird gelesen: {}", src));
        let prev_root = prev_snap.as_ref().and_then(|p| p.roots.iter().find(|n| n.name == *stored));
        let m = match walk.src.stat(ctx, src) {
            Ok(m) => m,
            Err(e) => {
                walk.warn(src, &e);
                continue;
            }
        };
        match walk.node(src, stored, m, prev_root) {
            Ok(Some(n)) => roots.push(n),
            Ok(None) => {}
            Err(CasError::Cancelled) => return Err(String::from("Abgebrochen")),
            Err(e) => return Err(cas_err(e)),
        }
    }
    let mut stats = walk.stats;
    let walk_changed = walk.changed;
    let ws = walk.w.finish().map_err(cas_err)?;
    stats.new_bytes = ws.new_stored;
    stats.new_chunks = ws.new_chunks;
    stats.new_raw = ws.new_raw;
    let s = walk.session;
    s.files_total = stats.files;
    s.files_done = stats.files;
    s.bytes_total = stats.bytes;
    s.bytes_read = stats.bytes;
    s.bytes_new = ws.new_stored;
    if ws.new_stored > 0 {
        s.dedup_x100 = (ws.raw.saturating_mul(100) / ws.new_raw.max(1)).min(u32::MAX as u64) as u32;
        s.compress_x100 = (ws.new_raw.saturating_mul(100) / ws.new_stored.max(1)).min(u32::MAX as u64) as u32;
    }
    log(ctx, s, LogLevel::Info, &format!("{} Dateien gelesen, {} neue Daten nach Dedupe und Kompression", bc_proto::count_text(stats.files), size(ws.new_stored)));
    if roots.is_empty() {
        return Err(String::from("Keine der Quellen war lesbar"));
    }
    let changed = walk_changed || prev_snap.as_ref().is_none_or(|p| p.roots.len() != roots.len());
    if skip_unchanged && !changed {
        log(ctx, s, LogLevel::Info, "Keine Änderungen seit dem letzten Restore-Point: kein neuer Punkt");
        s.point_id = prev.as_ref().map_or(String::new(), |p| p.id.clone());
        return Ok(String::from("Keine Änderungen"));
    }
    let t = now();
    let imm = repo.immutable_days();
    let snap = Snapshot {
        id: new_id(),
        created: t,
        job_id: job.id.clone(),
        job_name: job.name.clone(),
        host: host.name.clone(),
        roots,
        parent: prev.as_ref().map_or(String::new(), |p| p.id.clone()),
        full,
        stats,
        immutable_until: if imm > 0 { t + imm as i64 * DAY } else { 0 },
        meta: bc_proto::j::obj([("session", Value::string(s.id.as_str())), ("created_ns", Value::i64(plat::unix_time_ns()))]),
    };
    repo.save_snapshot(&snap, &ctx.keys.signing).map_err(cas_err)?;
    let point = point_of(&snap, &r.id, job.encrypted);
    let _ = ctx.db.put(C_POINTS, &point.id, point.to_json());
    s.point_id = point.id.clone();
    log(ctx, s, LogLevel::Info, "Restore-Point signiert und veröffentlicht");
    // retention
    match if keep_rules { retention(ctx, job, &repo, &r.id, s) } else { Ok(0) } {
        Ok(0) => {}
        Ok(n) => {
            log(ctx, s, LogLevel::Info, &format!("Aufbewahrung angewendet: {} alte Restore-Points entfernt", n));
            if let Ok(g) = repo.gc(&mut |_, _| true) {
                log(ctx, s, LogLevel::Info, &format!("{} freigegeben", size(g.bytes_freed)));
            }
        }
        Err(e) => {
            s.warnings += 1;
            log(ctx, s, LogLevel::Warning, &format!("Aufbewahrung: {}", e));
        }
    }
    update_repo_size(ctx, &r.id, &repo);
    let msg = format!("{} Dateien, {} gelesen, {} neu gespeichert", bc_proto::count_text(stats.files), size(stats.bytes), size(ws.new_stored));
    if s.warnings > 0 { Ok(format!("{} · {} Warnungen", msg, s.warnings)) } else { Ok(msg) }
}

/// the points of a job the retention keeps (ids)
pub fn keep_set(points: &[RestorePoint], ret: &Retention, t: i64) -> BTreeSet<String> {
    let mut v: Vec<&RestorePoint> = points.iter().collect();
    v.sort_by_key(|p| core::cmp::Reverse(p.order()));
    let mut keep = BTreeSet::new();
    if let Some(newest) = v.first() {
        keep.insert(newest.id.clone()); // never the newest
    }
    for p in v.iter().take(ret.points as usize) {
        keep.insert(p.id.clone());
    }
    for p in v.iter().filter(|p| ret.days > 0 && t - p.created < ret.days as i64 * DAY) {
        keep.insert(p.id.clone());
    }
    // GFS: the newest point of each week / month / year, for so many of them
    let mut bucket = |n: u32, key: &dyn Fn(i64) -> i64| {
        let mut seen = BTreeSet::new();
        for p in &v {
            if seen.len() >= n as usize {
                break;
            }
            let k = key(p.created);
            if seen.insert(k) {
                keep.insert(p.id.clone());
            }
        }
    };
    bucket(ret.weekly, &|c| (c / DAY + 3).div_euclid(7)); // weeks start on Monday
    bucket(ret.monthly, &|c| {
        let d = datetime::Zone::utc().to_local(datetime::Timestamp::from_secs(c));
        d.year as i64 * 12 + d.month as i64
    });
    bucket(ret.yearly, &|c| datetime::Zone::utc().to_local(datetime::Timestamp::from_secs(c)).year as i64);
    keep
}

/// delete the points the retention no longer keeps -> how many went
fn retention(ctx: &Ctx, job: &Job, repo: &Repo, repo_id: &str, s: &Session) -> Result<usize, String> {
    let points: Vec<RestorePoint> = all::<RestorePoint>(&ctx.db, C_POINTS).into_iter().filter(|p| p.job_id == job.id && p.repository_id == repo_id).collect();
    let mut keep = keep_set(&points, &job.retention, now());
    if job.cdp && job.cdp_keep_hours > 0 {
        let from = now() - job.cdp_keep_hours as i64 * 3600;
        keep.extend(points.iter().filter(|p| p.created >= from).map(|p| p.id.clone()));
    }
    let mut n = 0;
    for p in points.iter().filter(|p| !keep.contains(&p.id)) {
        match repo.delete_snapshot(&p.id, now()) {
            Ok(()) => {
                let _ = ctx.db.delete(C_POINTS, &p.id);
                n += 1;
            }
            Err(CasError::Immutable(_)) => {} // stays until its time has come
            Err(CasError::NotFound(_)) => {
                let _ = ctx.db.delete(C_POINTS, &p.id);
            }
            Err(e) => log(ctx, s, LogLevel::Warning, &format!("Restore-Point {} bleibt: {}", p.id, e)),
        }
    }
    Ok(n)
}

// ------------------------------------------------------------------ browse

/// the node at `path` inside a snapshot (a root, or below one)
fn find_node(repo: &Repo, snap: &Snapshot, path: &str) -> Result<Node, String> {
    let norm = |s: &str| String::from(s.replace('\\', "/").trim_end_matches('/'));
    let p = norm(path);
    for root in &snap.roots {
        let r = norm(&root.name);
        if p == r {
            return Ok(root.clone());
        }
        if let Some(rest) = p.strip_prefix(&format!("{}/", r)) {
            let mut node = root.clone();
            for part in rest.split('/').filter(|x| !x.is_empty()) {
                let t = node.tree.ok_or("kein Ordner")?;
                let tree = repo.read_tree(&t).map_err(cas_err)?;
                node = tree.find(part).cloned().ok_or_else(|| format!("„{}“ nicht im Restore-Point", path))?;
            }
            return Ok(node);
        }
    }
    Err(format!("„{}“ nicht im Restore-Point", path))
}

fn entry(n: &Node) -> Entry {
    Entry {
        name: n.name.clone(),
        kind: match n.kind {
            NodeKind::File => EntryKind::File,
            NodeKind::Dir => EntryKind::Dir,
            NodeKind::Symlink => EntryKind::Link,
            NodeKind::Other => EntryKind::Other,
        },
        size: n.size,
        mtime: n.mtime_ns.div_euclid(1_000_000_000),
    }
}

pub fn browse_point(ctx: &Ctx, p: &RestorePoint, path: &str, offset: usize, _pass: Option<&str>) -> Result<Listing, String> {
    let (_, repo) = repo_of(ctx, &p.repository_id)?;
    let snap = repo.load_snapshot(&p.id).map_err(cas_err)?;
    if path.is_empty() {
        return Ok(Listing { path: String::new(), entries: snap.roots.iter().map(entry).collect(), more: false, offset: 0 });
    }
    let node = find_node(&repo, &snap, path)?;
    let tree = match node.tree {
        Some(t) => repo.read_tree(&t).map_err(cas_err)?,
        None => return Err(String::from("Das ist kein Ordner")),
    };
    let mut nodes: Vec<&Node> = tree.nodes.iter().collect();
    nodes.sort_by(|a, b| ((a.kind != NodeKind::Dir), a.name.to_lowercase()).cmp(&((b.kind != NodeKind::Dir), b.name.to_lowercase())));
    let more = nodes.len() > offset + 1000;
    Ok(Listing { path: String::from(path), entries: nodes.iter().skip(offset).take(1000).map(|n| entry(n)).collect(), more, offset: offset as u32 })
}

// ------------------------------------------------------------------ restore, verify, delete

pub use crate::restore::start_restore;

pub fn start_verify(ctx: &Arc<Ctx>, p: &RestorePoint, actor: &str) -> String {
    let job = rec::<Job>(ctx.get(C_JOBS, &p.job_id)).unwrap_or(Job { id: p.job_id.clone(), name: p.job_name.clone(), ..Job::default() });
    let mut session = new_session(ctx, SessionKind::Verify, &job, &p.id);
    let pp = p.clone();
    Ops::start(ctx, "point.verify", &format!("{}: Prüfung", p.job_name), actor, &p.id, &session.id.clone(), Box::new(move |o| {
        let ctx = o.ctx().clone();
        let r = verify(o, &ctx, &pp, &mut session);
        close_session(&ctx, &mut session, &r, o.cancelled());
        if r.is_ok() {
            let _ = ctx.db.update(C_POINTS, &pp.id, |v| {
                v.insert("verified", Value::i64(now()));
            });
        }
        r
    }))
}

fn verify(o: &OpCtl, ctx: &Ctx, p: &RestorePoint, s: &mut Session) -> Result<String, String> {
    let (_, repo) = repo_of(ctx, &p.repository_id)?;
    let snap = repo.load_snapshot(&p.id).map_err(cas_err)?;
    log(ctx, s, LogLevel::Info, "Signatur des Restore-Points geprüft");
    let mut ids = BTreeSet::new();
    repo.referenced(&snap, &mut ids).map_err(cas_err)?;
    let total = ids.len() as u64;
    s.files_total = snap.stats.files;
    let mut bad = 0u64;
    for (k, id) in ids.iter().enumerate() {
        if o.cancelled() {
            return Err(String::from("Abgebrochen"));
        }
        match repo.read_chunk(id) {
            Ok(c) => s.bytes_read += c.len() as u64,
            Err(e) => {
                bad += 1;
                s.errors += 1;
                log(ctx, s, LogLevel::Error, &cas_err(e));
            }
        }
        o.progress(k as u64 + 1, total);
    }
    s.bytes_total = s.bytes_read;
    if bad > 0 {
        return Err(format!("{} von {} Chunks sind beschädigt oder fehlen", bad, total));
    }
    Ok(format!("{} Chunks gelesen und geprüft ({})", total, size(s.bytes_read)))
}

pub fn delete_point(ctx: &Ctx, p: &RestorePoint) -> Result<(), String> {
    let (_, repo) = repo_of(ctx, &p.repository_id)?;
    match repo.delete_snapshot(&p.id, now()) {
        Ok(()) | Err(CasError::NotFound(_)) => {
            let _ = ctx.db.delete(C_POINTS, &p.id);
            let pts = all::<RestorePoint>(&ctx.db, C_POINTS).iter().filter(|x| x.job_id == p.job_id).count();
            let _ = ctx.db.update(C_JOBS, &p.job_id, |v| {
                v.insert("points", Value::u64(pts as u64));
            });
            Ok(())
        }
        Err(CasError::Immutable(t)) => Err(format!("unveränderlich bis {}", t)),
        Err(e) => Err(cas_err(e)),
    }
}

/// for restore.rs
pub(crate) fn find(repo: &Repo, snap: &Snapshot, path: &str) -> Result<Node, String> {
    find_node(repo, snap, path)
}

pub(crate) fn session_for(ctx: &Ctx, kind: SessionKind, job: &Job, point: &str) -> Session {
    new_session(ctx, kind, job, point)
}

pub(crate) fn finish_session(ctx: &Ctx, s: &mut Session, r: &Result<String, String>, cancelled: bool) {
    close_session(ctx, s, r, cancelled)
}

pub(crate) fn session_log(ctx: &Ctx, s: &Session, level: LogLevel, msg: &str) {
    log(ctx, s, level, msg)
}

pub(crate) fn save(ctx: &Ctx, s: &Session) {
    save_session(ctx, s)
}

pub(crate) fn repo_for(ctx: &Ctx, id: &str) -> Result<(Repository, Arc<Repo>), String> {
    repo_of(ctx, id)
}

pub(crate) fn err_text(e: CasError) -> String {
    cas_err(e)
}
