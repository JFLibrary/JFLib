// hosts -- the server itself and paired agents: listing, the shared folders (backup sources and
// restore targets), removal (the agent's certificate is revoked), browsing the file system
// inside the shared folders, connection checks, pairing codes, agent enrolment and heartbeats,
// the SSH push

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

use bc_proto::api::{code, HostUpdate};
use bc_proto::j::{self, Json, Value};
use bc_proto::model::{Entry, EntryKind, Host, HostKind, HostStatus, Listing, PairingCode};
use netio::http::Response;

use crate::api::Call;
use crate::ctx::{Ctx, C_HOSTS, C_JOBS, C_META, C_PAIRING};
use crate::ops::Ops;
use crate::util::{self, all, bad, conflict, err, field, list as list_resp, no_content, not_found, now, ok, rec};

/// agents count as offline after this many seconds without a heartbeat
pub const OFFLINE_AFTER: i64 = 120;
const PAGE: usize = 1000;

/// a host with its current status
pub fn live(mut h: Host) -> Host {
    if h.kind == HostKind::Server {
        h.status = HostStatus::Online;
        h.last_seen = now();
    } else if h.status == HostStatus::Online && now() - h.last_seen > OFFLINE_AFTER {
        h.status = HostStatus::Offline;
        h.message = String::from("Keine Verbindung zum Agenten");
    }
    h
}

pub fn list(ctx: &Ctx) -> Response {
    let mut v: Vec<Host> = all::<Host>(&ctx.db, C_HOSTS).into_iter().map(live).collect();
    v.sort_by(|a, b| (a.kind != HostKind::Server, a.name.to_lowercase()).cmp(&(b.kind != HostKind::Server, b.name.to_lowercase())));
    list_resp(&v)
}

pub fn get(ctx: &Ctx, c: &Call) -> Response {
    match rec::<Host>(ctx.get(C_HOSTS, &c.param("id"))) {
        Some(h) => ok(200, &live(h)),
        None => not_found("Host nicht gefunden"),
    }
}

/// whether a host's paths follow Windows rules
pub fn host_windows(h: &Host) -> bool {
    if h.kind == HostKind::Server { cfg!(windows) } else { h.os == bc_proto::model::Os::Windows }
}

fn clean_roots(roots: &[String], what: &str, win: bool) -> Result<Vec<String>, Response> {
    bc_fs::clean_roots(roots, win).map_err(|e| field(code::BAD_REQUEST, &e, what))
}

pub fn update(ctx: &Ctx, c: &Call) -> Response {
    let id = c.param("id");
    let Some(r) = HostUpdate::from_json(&c.body) else { return bad("Ungültige Anfrage") };
    let Some(mut h): Option<Host> = rec(ctx.get(C_HOSTS, &id)) else { return not_found("Host nicht gefunden") };
    let win = host_windows(&h);
    let roots = match clean_roots(&r.roots, "roots", win) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let restore_roots = match clean_roots(&r.restore_roots, "restore_roots", win) {
        Ok(v) => v,
        Err(e) => return e,
    };
    if !r.name.trim().is_empty() {
        h.name = String::from(r.name.trim());
    }
    // an agent keeps its folders itself: they change there first, the record follows its answer
    if h.kind == HostKind::Agent && (roots != h.roots || restore_roots != h.restore_roots) {
        match crate::agents::set_roots(ctx, &h, &roots, &restore_roots) {
            Ok(a) => {
                h.roots = a.roots;
                h.restore_roots = a.restore_roots;
            }
            Err(e) => return conflict(&format!("Die Freigaben konnten nicht auf den Agenten übertragen werden: {}", e)),
        }
    } else {
        h.roots = roots;
        h.restore_roots = restore_roots;
    }
    if ctx.db.put(C_HOSTS, &id, h.to_json()).is_err() {
        return util::internal("");
    }
    c.audit("host.update", &h.name, &format!("{} Sicherungs-, {} Wiederherstellungsordner", h.roots.len(), h.restore_roots.len()));
    ok(200, &live(h))
}

pub fn remove(ctx: &Ctx, c: &Call) -> Response {
    let id = c.param("id");
    let Some(h): Option<Host> = rec(ctx.get(C_HOSTS, &id)) else { return not_found("Host nicht gefunden") };
    if h.kind == HostKind::Server {
        return conflict("Der Backup-Server selbst kann nicht entfernt werden");
    }
    if all::<bc_proto::model::Job>(&ctx.db, C_JOBS).iter().any(|j| j.host_id == id) {
        return conflict("Jobs sichern diesen Host noch");
    }
    // the certificate is revoked: its fingerprint goes on the list the TLS layer checks
    let mut revoked: Vec<String> = ctx.db.get(C_META, "revoked").map(|v| j::Field::take(v.get("fingerprints"))).unwrap_or_default();
    if !h.fingerprint.is_empty() && !revoked.contains(&h.fingerprint) {
        revoked.push(h.fingerprint.clone());
    }
    let _ = ctx.db.write(|_, tx| {
        tx.delete(C_HOSTS, &id);
        tx.put(C_META, "revoked", j::obj([("fingerprints", j::Field::put(&revoked))]));
        Ok::<(), datastore::StoreError>(())
    });
    c.audit("host.remove", &h.name, "Zertifikat gesperrt");
    no_content()
}

/// whether `path` lies inside one of `roots` (no "..", same prefix up to a separator)
pub fn inside(roots: &[String], path: &str) -> bool {
    bc_fs::inside(roots, path)
}

fn entry_kind(k: plat::fs::FileKind) -> EntryKind {
    match k {
        plat::fs::FileKind::File => EntryKind::File,
        plat::fs::FileKind::Dir => EntryKind::Dir,
        plat::fs::FileKind::Symlink => EntryKind::Link,
        plat::fs::FileKind::Other => EntryKind::Other,
    }
}

/// a listing of a folder of this machine (directories first, by name; pages of 1000)
pub fn local_listing(path: &str, offset: usize) -> Option<Listing> {
    let mut items = plat::fs::read_dir(path)?;
    items.sort_by(|a, b| ((a.kind != plat::fs::FileKind::Dir), a.name.to_lowercase()).cmp(&((b.kind != plat::fs::FileKind::Dir), b.name.to_lowercase())));
    let more = items.len() > offset + PAGE;
    let entries = items
        .iter()
        .skip(offset)
        .take(PAGE)
        .map(|e| {
            let m = plat::fs::lstat(&plat::fs::join(path, &e.name));
            Entry { name: e.name.clone(), kind: entry_kind(e.kind), size: m.map_or(0, |m| m.size), mtime: m.map_or(0, |m| m.mtime_ns / 1_000_000_000) }
        })
        .collect();
    Some(Listing { path: String::from(path), entries, more, offset: offset as u32 })
}

pub fn browse(ctx: &Ctx, c: &Call) -> Response {
    let Some(h): Option<Host> = rec(ctx.get(C_HOSTS, &c.param("id"))) else { return not_found("Host nicht gefunden") };
    let path = c.query("path").unwrap_or_default();
    let offset = c.query("offset").and_then(|o| o.parse().ok()).unwrap_or(0usize);
    if path.is_empty() {
        // the shared folders themselves
        let entries = h.roots.iter().map(|r| Entry { name: r.clone(), kind: EntryKind::Dir, size: 0, mtime: 0 }).collect();
        return ok(200, &Listing { path: String::new(), entries, more: false, offset: 0 });
    }
    if !bc_fs::inside_os(&h.roots, &path, host_windows(&h)) {
        return err(403, code::FORBIDDEN, "Der Ordner ist für Sicherungen nicht freigegeben");
    }
    if h.kind == HostKind::Agent {
        return match crate::agents::browse(ctx, &h, &path, offset) {
            Ok(l) => ok(200, &l),
            Err(e) => err(503, code::BUSY, &e),
        };
    }
    match local_listing(&path, offset) {
        Some(l) => ok(200, &l),
        None => not_found("Der Ordner ist nicht lesbar"),
    }
}

pub fn check(ctx: &Arc<Ctx>, c: &Call) -> Response {
    let Some(h): Option<Host> = rec(ctx.get(C_HOSTS, &c.param("id"))) else { return not_found("Host nicht gefunden") };
    let hh = h.clone();
    let op = Ops::start(ctx, "host.check", &format!("Verbindung zu {} prüfen", h.name), &c.actor(), &h.id, "", Box::new(move |o| {
        if hh.kind == HostKind::Server {
            return Ok(String::from("Der Backup-Server selbst ist erreichbar"));
        }
        crate::agents::check(o.ctx(), &hh)
    }));
    c.audit("host.check", &h.name, "");
    util::accepted(&op)
}

/// a pairing code: 8 characters without look-alikes, 15 minutes, once
pub fn pairing(ctx: &Ctx, c: &Call) -> Response {
    let Ok(pc) = new_pairing(ctx, &c.actor()) else { return util::internal("") };
    c.audit("host.pairing", "", "Pairing-Code erzeugt (15 min gültig)");
    ok(201, &pc)
}

/// a new pairing code, kept as its BLAKE3 digest
pub fn new_pairing(ctx: &Ctx, by: &str) -> Result<PairingCode, ()> {
    const ALPHA: &[u8; 32] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
    let r: [u8; 8] = plat::random_bytes();
    let mut code = String::new();
    for (i, b) in r.iter().enumerate() {
        if i == 4 {
            code.push('-');
        }
        code.push(ALPHA[(b & 31) as usize] as char);
    }
    let expires = now() + 15 * 60;
    let key = crypto::hex(&crypto::blake3::hash(code.as_bytes()));
    ctx.db.put(C_PAIRING, &key, j::obj([("expires", Value::i64(expires)), ("by", Value::string(by))])).map_err(|_| ())?;
    Ok(PairingCode { code, expires, server_url: crate::agents::public_url(ctx), ca_fingerprint: crate::agents::ca_fingerprint(ctx) })
}

pub fn enroll(ctx: &Ctx, c: &Call) -> Response {
    crate::agents::enroll(ctx, c)
}

pub fn heartbeat(ctx: &Ctx, c: &Call) -> Response {
    crate::agents::heartbeat(ctx, c)
}

pub fn ssh_fingerprint(ctx: &Ctx, c: &Call) -> Response {
    crate::agents::ssh_fingerprint(ctx, c)
}

pub fn ssh_push(ctx: &Arc<Ctx>, c: &Call) -> Response {
    crate::agents::ssh_push(ctx, c)
}
