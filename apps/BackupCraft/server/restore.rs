// restore -- entries of a restore point back to their place or into a folder
//
// Rules (the user's role decides what is allowed):
//   - restore role: only into the folders of the setting restore.allowed_roots and the host's
//     restore folders, never to the original place, never replacing files
//   - operators and administrators: also to the original place and replacing files
//   - every name from the restore point is checked (no separators, no "..", no drive)
//   - no symbolic link on the way: a folder of the target path that is a link stops the
//     restore of that entry (a link planted in the target cannot redirect the data)
//   - files are written to "<name>.bcrestore-<random>" in their folder and renamed when
//     complete and synced; times, permissions and attributes follow
//   - existing files: skip, rename ("name (wiederhergestellt).ext", "... 2" ...) or replace
//   - on an agent the same rules hold: the agent creates folders without following links,
//     writes temporary files and renames them, and accepts only paths inside its shared
//     folders (backup and restore folders)
//
// mirror() makes a folder equal to entries of a point (replicas): unchanged files (same size,
// modification time within a second) stay, changed ones are replaced, and -- when removing is
// allowed (a replica's own folder) -- what the point does not have goes.

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

use bc_proto::agent::{RestoreCommit, RestoreDir, RestoreLink, READ_MAX};
use bc_proto::api::{Conflict, RestoreRequest};
use bc_proto::model::*;
use cas::{Node, NodeKind, Repo};

use crate::ctx::{Ctx, C_HOSTS, C_JOBS};
use crate::engine::{self, size};
use bc_fs::{absolute, inside_os, join_os, safe_name, Target};
use crate::ops::{OpCtl, Ops};
use crate::util::rec;

/// where an alternative restore may write: the server's setting applies to the server's own
/// file system, an agent's restore folders to the agent
pub(crate) fn allowed_roots(ctx: &Ctx, host: &Host) -> Vec<String> {
    let mut v: Vec<String> = Vec::new();
    if host.kind == HostKind::Server {
        v.extend(ctx.setting("restore.allowed_roots").lines().map(|l| String::from(l.trim())).filter(|l| !l.is_empty()));
    }
    v.extend(host.restore_roots.iter().cloned());
    v
}



fn server_host(ctx: &Ctx) -> Option<Host> {
    crate::util::all::<Host>(&ctx.db, C_HOSTS).into_iter().find(|h| h.kind == HostKind::Server)
}

/// the folder path of an item below an alternative target: "C:\Daten\x" -> "C\Daten\x",
/// "/srv/x" -> "srv/x"
pub(crate) fn relative_of(item: &str, win: bool) -> String {
    let t = item.replace('\\', "/");
    let t = t.trim_start_matches('/');
    let t = if t.len() >= 2 && t.as_bytes()[1] == b':' { format!("{}{}", &t[..1], &t[2..]) } else { String::from(t) };
    let sep = if win { "\\" } else { "/" };
    t.split('/').filter(|p| !p.is_empty() && *p != "." && *p != "..").collect::<Vec<_>>().join(sep)
}


pub fn start_restore(ctx: &Arc<Ctx>, p: &RestorePoint, req: RestoreRequest, user: Option<&User>, actor: &str) -> Result<String, (u16, String)> {
    let role = user.map_or(Role::Viewer, |u| u.role);
    let host = if req.host_id.is_empty() { server_host(ctx) } else { rec::<Host>(ctx.get(C_HOSTS, &req.host_id)) }.ok_or((400, String::from("Zielhost nicht gefunden")))?;
    let win = crate::hosts::host_windows(&host);
    let operator = role.allows(Role::Operator);
    if req.original && !operator {
        return Err((403, String::from("An den ursprünglichen Ort dürfen nur Operatoren wiederherstellen")));
    }
    if req.conflict == Conflict::Overwrite && !operator {
        return Err((403, String::from("Vorhandene Dateien ersetzen dürfen nur Operatoren")));
    }
    let target = String::from(req.target.trim());
    let roots = allowed_roots(ctx, &host);
    if !req.original {
        if !absolute(&target, win) {
            return Err((400, String::from("Bitte einen absoluten Zielordner angeben")));
        }
        if !inside_os(&roots, &target, win) {
            return Err((403, String::from("Dieser Zielordner ist für Wiederherstellungen nicht freigegeben")));
        }
    }
    let (_, repo) = engine::repo_for(ctx, &p.repository_id).map_err(|e| (409, e))?;
    let snap = repo.load_snapshot(&p.id).map_err(|e| (409, engine::err_text(e)))?;
    let items: Vec<String> = if req.items.iter().all(|i| i.trim().is_empty() || i.trim() == "/") {
        snap.roots.iter().map(|r| r.name.clone()).collect()
    } else {
        req.items.iter().map(|i| String::from(i.trim())).filter(|i| !i.is_empty()).collect()
    };
    let mut nodes = Vec::new();
    for it in &items {
        nodes.push((it.clone(), engine::find(&repo, &snap, it).map_err(|e| (400, e))?));
    }
    let job = rec::<Job>(ctx.get(C_JOBS, &p.job_id)).unwrap_or(Job { id: p.job_id.clone(), name: p.job_name.clone(), ..Job::default() });
    let mut session = engine::session_for(ctx, SessionKind::Restore, &job, &p.id);
    let title = format!("{}: Wiederherstellung", p.job_name);
    let conflict = req.conflict;
    let original = req.original;
    let op = Ops::start(ctx, "restore", &title, actor, &p.id, &session.id.clone(), Box::new(move |o| {
        let ctx = o.ctx().clone();
        engine::session_log(&ctx, &session, LogLevel::Info, &format!("Wiederherstellung gestartet: {} Einträge nach {} ({})", nodes.len(), if original { "Originalort" } else { target.as_str() }, host.name));
        let dst = if host.kind == HostKind::Agent { Dst::Agent(&host) } else { Dst::Local };
        if let Dst::Agent(h) = &dst {
            if let Err(e) = crate::agents::info(&ctx, h) {
                let res = Err(format!("Agent nicht erreichbar: {}", e));
                engine::finish_session(&ctx, &mut session, &res, false);
                return res;
            }
        }
        let root = if original { None } else { bc_fs::root_of(&roots, &target).map(String::from) };
        let mut r = Restorer { o, ctx: &ctx, repo: &repo, dst, win, root, conflict, s: &mut session, done: 0, total: 0, written: 0, skipped: 0, renamed: 0,
                               mirror: false, may_remove: false, unchanged: 0, removed: 0 };
        for (_, n) in &nodes {
            r.total += r.size_of(n);
        }
        r.s.bytes_total = r.total;
        let mut result = Ok(());
        for (item, n) in &nodes {
            let dest = if original { item.clone() } else { join_os(&target, &relative_of(item, win), win) };
            if let Err(e) = r.node(n, &dest) {
                result = Err(e);
                break;
            }
        }
        let summary = format!("{} Dateien wiederhergestellt ({}), {} übersprungen, {} umbenannt", r.written, size(r.done), r.skipped, r.renamed);
        let res = result.map(|_| summary);
        engine::finish_session(&ctx, &mut session, &res, o.cancelled());
        res
    }));
    Ok(op)
}

/// where a restore writes: this machine or an agent
enum Dst<'a> {
    Local,
    Agent(&'a Host),
}

struct Restorer<'a> {
    o: &'a OpCtl,
    ctx: &'a Ctx,
    repo: &'a Repo,
    dst: Dst<'a>,
    /// the target's path rules
    win: bool,
    /// the restore folder of an alternative target (no link below it is followed)
    root: Option<String>,
    conflict: Conflict,
    s: &'a mut Session,
    done: u64,
    total: u64,
    written: u64,
    skipped: u64,
    renamed: u64,
    /// mirror mode: keep unchanged files, replace changed ones; remove what the point lacks
    /// when may_remove
    mirror: bool,
    may_remove: bool,
    unchanged: u64,
    removed: u64,
}

/// what a mirror run did
pub struct MirrorStats {
    pub written: u64,
    pub unchanged: u64,
    pub removed: u64,
    pub bytes: u64,
    pub files: u64,
}

/// make folders equal to entries of a point: (node, destination) pairs on a host; `root`: the
/// shared folder the destinations lie in (no link below it is followed)
pub(crate) fn mirror(o: &OpCtl, ctx: &Ctx, repo: &Repo, host: &Host, root: Option<String>, pairs: &[(Node, String)], may_remove: bool, s: &mut Session) -> Result<MirrorStats, String> {
    let win = crate::hosts::host_windows(host);
    let dst = if host.kind == HostKind::Agent { Dst::Agent(host) } else { Dst::Local };
    let mut r = Restorer { o, ctx, repo, dst, win, root, conflict: Conflict::Overwrite, s, done: 0, total: 0, written: 0, skipped: 0, renamed: 0,
                           mirror: true, may_remove, unchanged: 0, removed: 0 };
    for (n, _) in pairs {
        r.total += r.size_of(n);
    }
    r.s.bytes_total = r.total;
    for (n, dest) in pairs {
        r.node(n, dest)?;
    }
    Ok(MirrorStats { written: r.written, unchanged: r.unchanged, removed: r.removed, bytes: r.done, files: r.written + r.unchanged })
}

/// same file: size, and the modification time within a second (file systems round)
fn same_file(size: u64, mtime_ns: i64, n: &Node) -> bool {
    size == n.size && (mtime_ns - n.mtime_ns).abs() <= 1_000_000_000
}

impl Restorer<'_> {
    fn size_of(&self, n: &Node) -> u64 {
        match (n.kind, n.tree) {
            (NodeKind::Dir, Some(t)) => self.repo.read_tree(&t).map_or(0, |tree| tree.nodes.iter().map(|c| self.size_of(c)).sum()),
            (NodeKind::File, _) => n.size,
            _ => 0,
        }
    }

    fn warn(&mut self, path: &str, what: &str) {
        self.s.warnings += 1;
        engine::session_log(self.ctx, self.s, LogLevel::Warning, &format!("{}: {}", path, what));
    }

    /// the destination of an entry that exists already -> None: skip it
    fn resolve(&mut self, dest: &str) -> Option<String> {
        match bc_fs::free_target(dest, self.conflict.as_str()) {
            (Target::Free, p) => Some(p),
            (Target::Renamed, p) => {
                self.renamed += 1;
                Some(p)
            }
            (Target::Skip, _) => {
                self.skipped += 1;
                None
            }
        }
    }

    fn node(&mut self, n: &Node, dest: &str) -> Result<(), String> {
        if self.o.cancelled() {
            return Err(String::from("Abgebrochen"));
        }
        if let Dst::Agent(h) = self.dst {
            return self.node_agent(h, n, dest);
        }
        if let Some(root) = &self.root {
            if let Err(e) = bc_fs::no_links(root, dest, false) {
                self.warn(dest, &e);
                return Ok(());
            }
        }
        let parent = plat::fs::parent(dest);
        if !parent.is_empty() {
            if let Err(e) = bc_fs::ensure_dir(parent) {
                self.warn(dest, &e);
                return Ok(());
            }
        }
        if self.mirror && self.may_remove {
            // an entry of another kind where this one goes: it goes first
            if let Some(m) = plat::fs::lstat(dest) {
                let fits = match n.kind {
                    NodeKind::Dir => m.is_dir() && !m.is_symlink(),
                    NodeKind::File => m.is_file(),
                    _ => false,
                };
                if !fits {
                    match bc_fs::remove_tree(dest) {
                        Ok(()) => self.removed += 1,
                        Err(e) => self.warn(dest, &e),
                    }
                }
            }
        }
        match n.kind {
            NodeKind::Dir => {
                if let Err(e) = bc_fs::ensure_dir(dest) {
                    self.warn(dest, &e);
                    return Ok(());
                }
                let tree = match n.tree {
                    Some(t) => self.repo.read_tree(&t).map_err(engine::err_text)?,
                    None => return Ok(()),
                };
                if self.mirror && self.may_remove {
                    for e in plat::fs::read_dir(dest).unwrap_or_default() {
                        if tree.find(&e.name).is_none() {
                            match bc_fs::remove_tree(&plat::fs::join(dest, &e.name)) {
                                Ok(()) => self.removed += 1,
                                Err(x) => self.warn(dest, &x),
                            }
                        }
                    }
                }
                for c in &tree.nodes {
                    if !safe_name(&c.name, self.win) {
                        self.warn(dest, &format!("unzulässiger Name „{}“ übersprungen", c.name));
                        continue;
                    }
                    self.node(c, &plat::fs::join(dest, &c.name))?;
                }
                bc_fs::apply_meta(dest, n.atime_ns, n.mtime_ns, n.mode, 0);
            }
            NodeKind::File => {
                if self.mirror {
                    if let Some(m) = plat::fs::lstat(dest).filter(|m| m.is_file()) {
                        if same_file(m.size, m.mtime_ns, n) {
                            self.unchanged += 1;
                            self.done += n.size;
                            self.o.progress(self.done, self.total.max(self.done));
                            return Ok(());
                        }
                    }
                }
                let Some(dest) = self.resolve(dest) else { return Ok(()) };
                self.file(n, &dest)?;
            }
            NodeKind::Symlink => {
                if self.mirror && plat::fs::read_link(dest).is_some_and(|l| l == n.link) {
                    return Ok(());
                }
                let Some(dest) = self.resolve(dest) else { return Ok(()) };
                if let Err(e) = bc_fs::make_link(&n.link, &dest, true, n.attrs & bc_fs::ATTR_DIRECTORY != 0) {
                    self.warn(&dest, &e);
                }
            }
            NodeKind::Other => self.warn(dest, "Sonderdatei übersprungen"),
        }
        Ok(())
    }

    fn file(&mut self, n: &Node, dest: &str) -> Result<(), String> {
        let tmp = bc_fs::temp_name(dest);
        let lp = plat::fs::long_path(&tmp);
        let Some(f) = plat::File::create_new(&lp) else {
            self.warn(dest, "kann nicht geschrieben werden");
            return Ok(());
        };
        let mut off = 0u64;
        for id in &n.chunks {
            if self.o.cancelled() {
                drop(f);
                plat::delete(&lp);
                return Err(String::from("Abgebrochen"));
            }
            let data = match self.repo.read_chunk(id) {
                Ok(d) => d,
                Err(e) => {
                    drop(f);
                    plat::delete(&lp);
                    self.s.errors += 1;
                    engine::session_log(self.ctx, self.s, LogLevel::Error, &format!("{}: {}", dest, engine::err_text(e)));
                    return Ok(());
                }
            };
            if f.write_at(off, &data) != Some(data.len()) {
                drop(f);
                plat::delete(&lp);
                self.warn(dest, "Schreibfehler (Speicher voll?)");
                return Ok(());
            }
            off += data.len() as u64;
            self.done += data.len() as u64;
            self.o.progress(self.done, self.total.max(self.done));
        }
        if let Err(e) = bc_fs::finish_file(f, &tmp, dest, n.atime_ns, n.mtime_ns, n.mode, n.attrs) {
            self.warn(dest, &e);
            return Ok(());
        }
        self.file_done();
        Ok(())
    }

    fn file_done(&mut self) {
        self.written += 1;
        self.s.files_done = self.written;
        self.s.bytes_read = self.done;
        if self.written % 200 == 0 {
            engine::save(self.ctx, self.s);
        }
    }

    /// an entry onto an agent (the agent enforces its folders and never follows links)
    fn node_agent(&mut self, h: &Host, n: &Node, dest: &str) -> Result<(), String> {
        let conflict = String::from(self.conflict.as_str());
        let existing = if self.mirror { crate::agents::stat(self.ctx, h, dest).ok() } else { None };
        if self.may_remove {
            if let Some(e) = &existing {
                let fits = match n.kind {
                    NodeKind::Dir => e.kind == "dir",
                    NodeKind::File => e.kind == "file",
                    _ => false,
                };
                if !fits {
                    match crate::agents::restore_remove(self.ctx, h, dest) {
                        Ok(()) => self.removed += 1,
                        Err(x) => self.warn(dest, &x),
                    }
                }
            }
        }
        match n.kind {
            NodeKind::Dir => {
                let d = RestoreDir { path: String::from(dest), atime_ns: 0, mtime_ns: 0, mode: 0 };
                if let Err(e) = crate::agents::restore_mkdir(self.ctx, h, &d) {
                    self.warn(dest, &e);
                    return Ok(());
                }
                let tree = match n.tree {
                    Some(t) => self.repo.read_tree(&t).map_err(engine::err_text)?,
                    None => return Ok(()),
                };
                if self.mirror && self.may_remove {
                    let mut off = 0;
                    let mut extra = Vec::new();
                    while let Ok(l) = crate::agents::list(self.ctx, h, dest, off) {
                        let k = l.entries.len();
                        extra.extend(l.entries.into_iter().filter(|e| tree.find(&e.name).is_none()).map(|e| e.name));
                        off += k;
                        if !l.more || k == 0 {
                            break;
                        }
                    }
                    for name in extra {
                        match crate::agents::restore_remove(self.ctx, h, &join_os(dest, &name, self.win)) {
                            Ok(()) => self.removed += 1,
                            Err(x) => self.warn(dest, &x),
                        }
                    }
                }
                for c in &tree.nodes {
                    if !safe_name(&c.name, self.win) {
                        self.warn(dest, &format!("unzulässiger Name „{}“ übersprungen", c.name));
                        continue;
                    }
                    self.node_agent(h, c, &join_os(dest, &c.name, self.win))?;
                    if self.o.cancelled() {
                        return Err(String::from("Abgebrochen"));
                    }
                }
                // the folder's times last (its files changed them)
                let d = RestoreDir { path: String::from(dest), atime_ns: n.atime_ns, mtime_ns: n.mtime_ns, mode: n.mode };
                let _ = crate::agents::restore_mkdir(self.ctx, h, &d);
            }
            NodeKind::File => {
                if existing.as_ref().is_some_and(|e| e.kind == "file" && same_file(e.size, e.mtime_ns, n)) {
                    self.unchanged += 1;
                    self.done += n.size;
                    self.o.progress(self.done, self.total.max(self.done));
                    return Ok(());
                }
                self.file_agent(h, n, dest, &conflict)?
            }
            NodeKind::Symlink => {
                let l = RestoreLink { path: String::from(dest), target: n.link.clone(), conflict, dir: n.attrs & bc_fs::ATTR_DIRECTORY != 0 };
                if let Err(e) = crate::agents::restore_symlink(self.ctx, h, &l) {
                    self.warn(dest, &e);
                }
            }
            NodeKind::Other => self.warn(dest, "Sonderdatei übersprungen"),
        }
        Ok(())
    }

    fn file_agent(&mut self, h: &Host, n: &Node, dest: &str, conflict: &str) -> Result<(), String> {
        let opened = match crate::agents::restore_open(self.ctx, h, dest, conflict) {
            Ok(x) => x,
            Err(e) => {
                self.warn(dest, &e);
                return Ok(());
            }
        };
        if opened.skip {
            self.skipped += 1;
            return Ok(());
        }
        if opened.dest != dest {
            self.renamed += 1;
        }
        let mut off = 0u64;
        let mut buf: Vec<u8> = Vec::new();
        let flush = |r: &mut Self, buf: &mut Vec<u8>, off: &mut u64| -> Result<(), String> {
            if buf.is_empty() {
                return Ok(());
            }
            crate::agents::restore_write(r.ctx, h, &opened.tmp, *off, buf)?;
            *off += buf.len() as u64;
            r.done += buf.len() as u64;
            r.o.progress(r.done, r.total.max(r.done));
            buf.clear();
            Ok(())
        };
        for id in &n.chunks {
            if self.o.cancelled() {
                crate::agents::restore_abort(self.ctx, h, &opened.tmp);
                return Err(String::from("Abgebrochen"));
            }
            let data = match self.repo.read_chunk(id) {
                Ok(d) => d,
                Err(e) => {
                    crate::agents::restore_abort(self.ctx, h, &opened.tmp);
                    self.s.errors += 1;
                    engine::session_log(self.ctx, self.s, LogLevel::Error, &format!("{}: {}", dest, engine::err_text(e)));
                    return Ok(());
                }
            };
            if !buf.is_empty() && buf.len() + data.len() > READ_MAX as usize {
                if let Err(e) = flush(self, &mut buf, &mut off) {
                    crate::agents::restore_abort(self.ctx, h, &opened.tmp);
                    self.warn(dest, &e);
                    return Ok(());
                }
            }
            buf.extend_from_slice(&data);
        }
        if let Err(e) = flush(self, &mut buf, &mut off) {
            crate::agents::restore_abort(self.ctx, h, &opened.tmp);
            self.warn(dest, &e);
            return Ok(());
        }
        let c = RestoreCommit { tmp: opened.tmp.clone(), dest: opened.dest.clone(), atime_ns: n.atime_ns, mtime_ns: n.mtime_ns, mode: n.mode, attrs: n.attrs };
        if let Err(e) = crate::agents::restore_commit(self.ctx, h, &c) {
            crate::agents::restore_abort(self.ctx, h, &opened.tmp);
            self.warn(dest, &e);
            return Ok(());
        }
        self.file_done();
        Ok(())
    }
}
