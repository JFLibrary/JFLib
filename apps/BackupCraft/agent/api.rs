// api -- what the server may ask of the agent (bc_proto::agent), over mutual TLS
//
// Paths: absolute, inside a shared folder, no link below that folder on the way.  Reading,
// listing and writing: backup and restore folders (reading a restore folder: a replica's
// failback).  Removing (replica
// mirrors): the restore folders only, never a folder itself, links as links.  Restored files are
// written to temporary files the agent itself names and remembers; write / commit / abort
// accept only those.

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use bc_proto::agent::*;
use bc_proto::api::{code, ApiError};
use bc_proto::j::{self, Json};
use netio::http::{Request, Response};
use plat::sync::Mutex;

use crate::config::{self, Config};

/// the changes in the backup folders (CDP), numbered
#[derive(Default)]
pub struct ChangeLog {
    pub seq: u64,
    pub entries: alloc::collections::VecDeque<(u64, String)>,
    /// changes up to this number are lost (overflow, or dropped from a long log)
    pub lost: u64,
}

pub struct State {
    pub dir: String,
    pub cfg: Mutex<Config>,
    pub changes: Mutex<ChangeLog>,
    pub changed: plat::sync::Condvar,
    /// open restores: temporary file -> its handle (written through it only, so a link swapped
    /// in for the name cannot redirect the data), destination, start
    pub temps: Mutex<BTreeMap<String, Temp>>,
}

pub struct Temp {
    file: plat::File,
    dest: String,
    at: i64,
}

fn err(status: u16, c: &str, m: &str) -> Response {
    Response::json(status, &ApiError { code: String::from(c), message: String::from(m), field: String::new() }.to_json())
}

fn ok<T: Json>(v: &T) -> Response {
    Response::json(200, &v.to_json())
}

fn done() -> Response {
    Response::json(200, &j::obj([("ok", j::Value::Bool(true))]))
}

fn forbidden(path: &str) -> Response {
    err(403, code::FORBIDDEN, &format!("{} liegt in keinem freigegebenen Ordner", path))
}

enum Access {
    Read,
    Write,
    /// the restore folders only
    Remove,
}

impl State {
    fn roots(&self, a: Access) -> Vec<String> {
        let c = self.cfg.lock();
        match a {
            Access::Read | Access::Write => c.roots.iter().chain(c.restore_roots.iter()).cloned().collect(),
            Access::Remove => c.restore_roots.clone(),
        }
    }

    /// the shared folder a path may be used in, with no link below it on the way (`last`: the
    /// path itself must not be a link either)
    fn check(&self, path: &str, a: Access, last: bool) -> Result<String, Response> {
        if !bc_fs::absolute(path, cfg!(windows)) {
            return Err(err(400, code::BAD_REQUEST, "Pfad nicht absolut"));
        }
        let roots = self.roots(a);
        let root = bc_fs::root_of(&roots, path).ok_or_else(|| forbidden(path))?;
        bc_fs::no_links(root, path, last).map_err(|e| err(403, code::FORBIDDEN, &e))?;
        Ok(String::from(root))
    }
}

fn q(req: &Request, k: &str) -> String {
    req.query(k).unwrap_or_default()
}

fn q_u64(req: &Request, k: &str) -> u64 {
    req.query(k).and_then(|v| v.parse().ok()).unwrap_or(0)
}

fn body<T: Json>(req: &Request) -> Option<T> {
    T::from_json(&j::parse(&req.body)?)
}

pub fn handle(st: &State, req: &Request) -> Response {
    if req.client_id.is_none() {
        return err(401, code::UNAUTHORIZED, "nur der Backup-Server");
    }
    match (req.method.as_str(), req.path()) {
        ("GET", "/v1/info") => info(st),
        ("GET", "/v1/list") => list(st, req),
        ("GET", "/v1/stat") => stat(st, req),
        ("GET", "/v1/read") => read(st, req),
        ("PUT", "/v1/roots") => roots(st, req),
        ("POST", "/v1/restore/open") => restore_open(st, req),
        ("POST", "/v1/restore/write") => restore_write(st, req),
        ("POST", "/v1/restore/commit") => restore_commit(st, req),
        ("POST", "/v1/restore/mkdir") => restore_mkdir(st, req),
        ("POST", "/v1/restore/symlink") => restore_symlink(st, req),
        ("POST", "/v1/restore/abort") => restore_abort(st, req),
        ("POST", "/v1/restore/remove") => restore_remove(st, req),
        ("GET", "/v1/changes") => changes(st, req),
        _ => err(404, code::NOT_FOUND, "unbekannt"),
    }
}

pub fn info_of(c: &Config) -> AgentInfo {
    AgentInfo {
        host_id: c.host_id.clone(),
        name: c.name.clone(),
        os: crate::os(),
        platform: crate::platform(),
        version: String::from(bc_proto::VERSION),
        roots: c.roots.clone(),
        restore_roots: c.restore_roots.clone(),
    }
}

fn info(st: &State) -> Response {
    ok(&info_of(&st.cfg.lock()))
}

fn list(st: &State, req: &Request) -> Response {
    let path = q(req, "path");
    if let Err(r) = st.check(&path, Access::Write, true) {
        return r;
    }
    let offset = q_u64(req, "offset") as usize;
    let limit = req.query("limit").and_then(|v| v.parse().ok()).unwrap_or(1000usize).clamp(1, 5000);
    match bc_fs::listing(&path, offset, limit) {
        Some(l) => ok(&l),
        None => err(404, code::NOT_FOUND, &format!("{}: Ordner nicht lesbar", path)),
    }
}

fn stat(st: &State, req: &Request) -> Response {
    let path = q(req, "path");
    if let Err(r) = st.check(&path, Access::Write, false) {
        return r;
    }
    let name = &path[plat::fs::parent(&path).len()..];
    match bc_fs::entry(&path, name.trim_start_matches(['/', '\\'])) {
        Some(e) => ok(&e),
        None => err(404, code::NOT_FOUND, &format!("{}: nicht vorhanden oder nicht lesbar", path)),
    }
}

fn read(st: &State, req: &Request) -> Response {
    let path = q(req, "path");
    if let Err(r) = st.check(&path, Access::Read, true) {
        return r;
    }
    if !plat::fs::lstat(&path).is_some_and(|m| m.is_file()) {
        return err(400, code::BAD_REQUEST, &format!("{}: keine Datei", path));
    }
    let off = q_u64(req, "offset");
    let len = q_u64(req, "len").clamp(1, READ_MAX) as usize;
    let Some(f) = plat::File::open(&plat::fs::long_path(&path), plat::FO_READ) else {
        return err(409, code::CONFLICT, "Datei nicht lesbar (gesperrt oder keine Berechtigung)");
    };
    let mut buf = alloc::vec![0u8; len];
    let mut got = 0;
    while got < len {
        match f.read_at(off + got as u64, &mut buf[got..]) {
            Some(0) => break,
            Some(n) => got += n,
            None => return err(409, code::CONFLICT, &format!("Lesefehler in {}", path)),
        }
    }
    buf.truncate(got);
    Response::bytes(200, "application/octet-stream", buf)
}

fn roots(st: &State, req: &Request) -> Response {
    let Some(r) = body::<AgentRoots>(req) else { return err(400, code::BAD_REQUEST, "Ungültige Anfrage") };
    let mut c = st.cfg.lock();
    if !c.remote_roots {
        return err(403, code::FORBIDDEN, "Die Freigaben dieses Agenten sind gesperrt (auf dem Agenten: bc-agent roots --unlock)");
    }
    let win = cfg!(windows);
    let (roots, restore_roots) = match (bc_fs::clean_roots(&r.roots, win), bc_fs::clean_roots(&r.restore_roots, win)) {
        (Ok(a), Ok(b)) => (a, b),
        (Err(e), _) | (_, Err(e)) => return err(400, code::BAD_REQUEST, &e),
    };
    let mut next = c.clone();
    next.roots = roots;
    next.restore_roots = restore_roots;
    if let Err(e) = config::save(&st.dir, &next) {
        return err(500, code::INTERNAL, &e);
    }
    *c = next;
    crate::log(&format!("Freigaben vom Server geändert: {} Sicherungs-, {} Wiederherstellungsordner", c.roots.len(), c.restore_roots.len()));
    ok(&AgentRoots { roots: c.roots.clone(), restore_roots: c.restore_roots.clone() })
}

/// temporary files of restores that never finished (older than six hours)
fn drop_stale(st: &State) {
    let t = crate::now();
    let mut temps = st.temps.lock();
    let old: Vec<String> = temps.iter().filter(|(_, x)| t - x.at > 6 * 3600).map(|(k, _)| k.clone()).collect();
    for k in old {
        temps.remove(&k);
        plat::delete(&plat::fs::long_path(&k));
    }
}

fn restore_open(st: &State, req: &Request) -> Response {
    let Some(r) = body::<RestoreOpen>(req) else { return err(400, code::BAD_REQUEST, "Ungültige Anfrage") };
    if let Err(x) = st.check(&r.path, Access::Write, false) {
        return x;
    }
    drop_stale(st);
    let parent = plat::fs::parent(&r.path);
    if let Err(e) = bc_fs::ensure_dir(parent) {
        return err(409, code::CONFLICT, &e);
    }
    let (t, dest) = bc_fs::free_target(&r.path, &r.conflict);
    if t == bc_fs::Target::Skip {
        return ok(&RestoreOpened { skip: true, tmp: String::new(), dest });
    }
    if plat::fs::lstat(&dest).is_some_and(|m| m.is_dir()) {
        return err(409, code::CONFLICT, &format!("{} ist ein Ordner", dest));
    }
    let tmp = bc_fs::temp_name(&dest);
    let Some(file) = plat::File::create_new(&plat::fs::long_path(&tmp)) else {
        return err(409, code::CONFLICT, &format!("{}: kann nicht geschrieben werden", dest));
    };
    st.temps.lock().insert(tmp.clone(), Temp { file, dest: dest.clone(), at: crate::now() });
    ok(&RestoreOpened { skip: false, tmp, dest })
}

fn restore_write(st: &State, req: &Request) -> Response {
    let tmp = q(req, "tmp");
    let off = q_u64(req, "offset");
    let temps = st.temps.lock();
    let Some(t) = temps.get(&tmp) else {
        return err(404, code::NOT_FOUND, "unbekannte Wiederherstellung");
    };
    if t.file.write_at(off, &req.body) != Some(req.body.len()) {
        return err(409, code::CONFLICT, "Schreibfehler (Speicher voll?)");
    }
    done()
}

fn restore_commit(st: &State, req: &Request) -> Response {
    let Some(c) = body::<RestoreCommit>(req) else { return err(400, code::BAD_REQUEST, "Ungültige Anfrage") };
    let t = {
        let mut temps = st.temps.lock();
        if temps.get(&c.tmp).is_none_or(|t| t.dest != c.dest) {
            return err(404, code::NOT_FOUND, "unbekannte Wiederherstellung");
        }
        temps.remove(&c.tmp)
    };
    let Some(t) = t else { return err(404, code::NOT_FOUND, "unbekannte Wiederherstellung") };
    // the way to the destination is checked again: it may have changed meanwhile
    if let Err(x) = st.check(&c.dest, Access::Write, false) {
        drop(t);
        plat::delete(&plat::fs::long_path(&c.tmp));
        return x;
    }
    match bc_fs::finish_file(t.file, &c.tmp, &c.dest, c.atime_ns, c.mtime_ns, c.mode, c.attrs) {
        Ok(()) => done(),
        Err(e) => err(409, code::CONFLICT, &format!("{}: {}", c.dest, e)),
    }
}

/// what changed after `since` (waiting up to `wait` ms for something); since = u64::MAX: only
/// the current number
fn changes(st: &State, req: &Request) -> Response {
    let since = req.query("since").and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
    let wait = q_u64(req, "wait").min(25_000) as u32;
    let mut g = st.changes.lock();
    if since == u64::MAX {
        return ok(&AgentChanges { seq: g.seq, paths: Vec::new(), overflow: false });
    }
    if g.seq <= since && wait > 0 {
        g = st.changed.wait_timeout(g, Some(wait)).0;
    }
    let paths: Vec<String> = g.entries.iter().filter(|(n, _)| *n > since).map(|(_, p)| p.clone()).take(50_000).collect();
    ok(&AgentChanges { seq: g.seq, overflow: g.lost > since || paths.len() >= 50_000, paths })
}

/// record changes of the watcher
pub fn note_changes(st: &State, c: plat::watch::Changes) {
    if c.paths.is_empty() && !c.overflow {
        return;
    }
    let mut g = st.changes.lock();
    for p in c.paths {
        g.seq += 1;
        let n = g.seq;
        g.entries.push_back((n, p));
        if g.entries.len() > 100_000 {
            if let Some((k, _)) = g.entries.pop_front() {
                g.lost = g.lost.max(k);
            }
        }
    }
    if c.overflow {
        g.seq += 1;
        g.lost = g.seq;
    }
    drop(g);
    st.changed.notify_all();
}

fn restore_remove(st: &State, req: &Request) -> Response {
    let Some(d) = body::<RemovePath>(req) else { return err(400, code::BAD_REQUEST, "Ungültige Anfrage") };
    let root = match st.check(&d.path, Access::Remove, false) {
        Ok(r) => r,
        Err(x) => return x,
    };
    if bc_fs::inside(&[d.path.clone()], &root) {
        return err(403, code::FORBIDDEN, "ein freigegebener Ordner selbst wird nicht gelöscht");
    }
    match bc_fs::remove_tree(&d.path) {
        Ok(()) => done(),
        Err(e) => err(409, code::CONFLICT, &e),
    }
}

fn restore_mkdir(st: &State, req: &Request) -> Response {
    let Some(d) = body::<RestoreDir>(req) else { return err(400, code::BAD_REQUEST, "Ungültige Anfrage") };
    if let Err(x) = st.check(&d.path, Access::Write, true) {
        return x;
    }
    if let Err(e) = bc_fs::ensure_dir(&d.path) {
        return err(409, code::CONFLICT, &e);
    }
    bc_fs::apply_meta(&d.path, d.atime_ns, d.mtime_ns, d.mode, 0);
    done()
}

fn restore_symlink(st: &State, req: &Request) -> Response {
    let Some(l) = body::<RestoreLink>(req) else { return err(400, code::BAD_REQUEST, "Ungültige Anfrage") };
    if let Err(x) = st.check(&l.path, Access::Write, false) {
        return x;
    }
    if let Err(e) = bc_fs::ensure_dir(plat::fs::parent(&l.path)) {
        return err(409, code::CONFLICT, &e);
    }
    let (t, dest) = bc_fs::free_target(&l.path, &l.conflict);
    if t == bc_fs::Target::Skip {
        return done();
    }
    match bc_fs::make_link(&l.target, &dest, true, l.dir) {
        Ok(()) => done(),
        Err(e) => err(409, code::CONFLICT, &format!("{}: {}", dest, e)),
    }
}

fn restore_abort(st: &State, req: &Request) -> Response {
    let tmp = q(req, "tmp");
    let t = st.temps.lock().remove(&tmp);
    if let Some(t) = t {
        drop(t);
        plat::delete(&plat::fs::long_path(&tmp));
    }
    done()
}
