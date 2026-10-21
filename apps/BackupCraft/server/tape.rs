// tape -- the tape library: virtual tapes (files of the library folder, format bc_tape), media
// pools, restore points written to tape (sets), restore and verify straight from tape, tape jobs
//
//   status / tapes / tapes_add / tape_update / tape_delete / start_erase / start_rescan
//   pools / pool_save / pool_delete
//   sets / set_browse / start_set_restore / start_set_verify
//   point_write(ctx, c)          POST /api/points/{id}/tape: a point to a pool
//   start_job(ctx, job, actor)   a tape job: the source job's newest point the pool lacks
//   after_backup(ctx, job)       tape jobs without a schedule follow their source job
//
// A set holds a restore point completely: the entries depth first (a folder before what is in
// it), each file with its data and its BLAKE3.  When a tape is full the set goes on on the next
// blank tape of the pool, or on a free one, which then joins the pool.  An appending pool fills
// its newest tape first.  Written tapes are write-protected for the pool's days; erasing and
// removing wait for that.  Encrypted pools give every set its own key: sealed with the master
// key in the set record and, when there is a recovery kit, sealed to it on the tape itself.
// The entries of a set are kept in <data>/tape/<set id>.cat for browsing; restore and verify
// read the tapes, never the repository.  Rescanning catalogs tapes that came back or came from
// elsewhere (an encrypted set of another server stays locked).  One tape operation runs at a
// time (DRIVE), like one drive.

use alloc::boxed::Box;
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

use bc_proto::agent::{RestoreCommit, RestoreDir, RestoreLink};
use bc_proto::api::{code, CartridgeRequest, CartridgeUpdate, Conflict, PoolRequest, RestoreRequest, TapeWriteRequest};
use bc_proto::j::{self, Json, Value};
use bc_proto::model::*;
use bc_tape::{self as bt, SetBegin, SetEnd, SetKey};
use bc_fs::{absolute, inside_os, join_os, Target};
use cas::{Node, NodeKind, Repo};
use netio::http::Response;
use plat::sync::Mutex;

use crate::api::Call;
use crate::ctx::{Ctx, C_HOSTS, C_JOBS, C_POINTS, C_POOLS, C_TAPES, C_TAPESETS};
use crate::engine::{self, size};
use crate::ops::{OpCtl, Ops};
use crate::util::{self, all, err, field, list, new_id, no_content, not_found, now, ok, rec};

const DAY: i64 = 86_400;
const GB: u64 = 1_000_000_000;
/// room kept on a tape for the end of a set part
const RESERVE: u64 = 64 * 1024;
/// a tape with less room than this counts as full
const MIN_ROOM: u64 = 16 << 20;

/// one tape operation at a time
static DRIVE: Mutex<()> = Mutex::new(());

// ------------------------------------------------------------------ records

/// the folder of the virtual tapes
pub fn library(ctx: &Ctx) -> String {
    let s = ctx.setting("tape.library");
    if s.trim().is_empty() { ctx.path("vtl") } else { String::from(s.trim()) }
}

fn tape_path(ctx: &Ctx, barcode: &str) -> String {
    plat::fs::join(&library(ctx), &format!("{}{}", barcode, bt::EXT))
}

fn catalog_path(ctx: &Ctx, set: &str) -> String {
    plat::fs::join(&ctx.path("tape"), &format!("{}.cat", set))
}

fn present(ctx: &Ctx, barcode: &str) -> bool {
    plat::fs::lstat(&tape_path(ctx, barcode)).is_some_and(|m| m.is_file())
}

fn cart(ctx: &Ctx, id: &str) -> Option<Cartridge> {
    rec(ctx.get(C_TAPES, id))
}

fn put_cart(ctx: &Ctx, c: &Cartridge) {
    let _ = ctx.db.put(C_TAPES, &c.id, c.to_json());
}

fn pool_of(ctx: &Ctx, id: &str) -> Option<MediaPool> {
    rec(ctx.get(C_POOLS, id))
}

fn set_of(ctx: &Ctx, id: &str) -> Option<TapeSet> {
    rec(ctx.get(C_TAPESETS, id))
}

/// a set record (its sealed key stays)
fn put_set(ctx: &Ctx, s: &TapeSet) {
    let key = ctx.db.get(C_TAPESETS, &s.id).map(|v| String::from(j::str_of(&v, "key"))).unwrap_or_default();
    let v = if key.is_empty() { s.to_json() } else { util::with(s.to_json(), "key", Value::string(key)) };
    let _ = ctx.db.put(C_TAPESETS, &s.id, v);
}

fn store_key(ctx: &Ctx, set: &str, key: &[u8; 32]) {
    let _ = ctx.db.update(C_TAPESETS, set, |v| {
        v.insert("key", Value::string(ctx.keys.seal(key, &format!("tapeset:{}", set))));
    });
}

fn key_of(ctx: &Ctx, set: &str) -> Option<[u8; 32]> {
    let v = ctx.db.get(C_TAPESETS, set)?;
    let k = ctx.keys.unseal(j::str_of(&v, "key"), &format!("tapeset:{}", set))?;
    k.try_into().ok()
}

/// the sets a tape holds go: erased or removed
fn drop_sets_on(ctx: &Ctx, barcode: &str) -> usize {
    let mut n = 0;
    for s in all::<TapeSet>(&ctx.db, C_TAPESETS).into_iter().filter(|s| s.tapes.iter().any(|t| t == barcode)) {
        let _ = ctx.db.delete(C_TAPESETS, &s.id);
        plat::delete(&catalog_path(ctx, &s.id));
        n += 1;
    }
    n
}

// ------------------------------------------------------------------ library and tapes

pub fn status(ctx: &Ctx) -> Response {
    let lib = library(ctx);
    let exists = plat::fs::lstat(&lib).is_some_and(|m| m.is_dir());
    let tapes: Vec<Cartridge> = all(&ctx.db, C_TAPES);
    let missing = tapes.iter().filter(|c| !present(ctx, &c.barcode)).count() as u32;
    ok(200, &TapeStatus {
        library: lib.clone(),
        ok: exists || tapes.is_empty(),
        message: if exists || tapes.is_empty() { String::new() } else { format!("Der Ordner {} fehlt", lib) },
        tapes: tapes.len() as u32,
        blank: tapes.iter().filter(|c| c.state == "blank").count() as u32,
        missing,
        used: tapes.iter().map(|c| c.used).sum(),
        capacity: tapes.iter().map(|c| c.capacity).sum(),
        sets: all::<TapeSet>(&ctx.db, C_TAPESETS).len() as u32,
    })
}

pub fn tapes(ctx: &Ctx) -> Response {
    let pools: BTreeMap<String, String> = all::<MediaPool>(&ctx.db, C_POOLS).into_iter().map(|p| (p.id, p.name)).collect();
    let mut v: Vec<Cartridge> = all(&ctx.db, C_TAPES);
    for c in &mut v {
        if !present(ctx, &c.barcode) {
            c.state = String::from("missing");
            c.message = String::from("Nicht in der Bibliothek (ausgelagert?)");
        }
        c.pool_name = pools.get(&c.pool_id).cloned().unwrap_or_default();
    }
    v.sort_by(|a, b| a.barcode.cmp(&b.barcode));
    list(&v)
}

pub fn tapes_add(ctx: &Ctx, c: &Call) -> Response {
    let Some(r) = CartridgeRequest::from_json(&c.body) else { return util::bad("Ungültige Anfrage") };
    if r.count == 0 || r.count > 100 {
        return field(code::BAD_REQUEST, "1 bis 100 Bänder auf einmal", "count");
    }
    if r.capacity_mb == 0 && (r.capacity_gb == 0 || r.capacity_gb > 100_000) {
        return field(code::BAD_REQUEST, "Die Kapazität liegt zwischen 1 und 100000 GB", "capacity_gb");
    }
    if r.capacity_mb > 0 && !(32..=1_000_000).contains(&r.capacity_mb) {
        return field(code::BAD_REQUEST, "Mindestens 32 MB", "capacity_mb");
    }
    let prefix = r.prefix.trim().to_ascii_uppercase();
    if !(2..=4).contains(&prefix.len()) || !prefix.chars().all(|x| x.is_ascii_uppercase()) {
        return field(code::BAD_REQUEST, "Das Barcode-Präfix hat 2 bis 4 Buchstaben", "prefix");
    }
    let pool = if r.pool_id.is_empty() { None } else { pool_of(ctx, &r.pool_id) };
    if !r.pool_id.is_empty() && pool.is_none() {
        return field(code::BAD_REQUEST, "Medienpool nicht gefunden", "pool_id");
    }
    let lib = library(ctx);
    if let Err(e) = bc_fs::ensure_dir(&lib) {
        return field(code::BAD_REQUEST, &format!("Bibliothek {}: {}", lib, e), "tape.library");
    }
    let t = now();
    let capacity = if r.capacity_mb > 0 { r.capacity_mb as u64 * 1_000_000 } else { r.capacity_gb as u64 * GB };
    let mut made = Vec::new();
    let mut n = 1u32;
    while made.len() < r.count as usize && n <= 9999 {
        let barcode = format!("{}{:04}V1", prefix, n);
        n += 1;
        if ctx.db.contains(C_TAPES, &barcode) || plat::fs::lstat(&tape_path(ctx, &barcode)).is_some() {
            continue;
        }
        let label = bt::Label { format: String::from(bt::FORMAT), barcode: barcode.clone(), capacity, created: t, library: ctx.server_name() };
        let tape = match bt::Tape::create(&tape_path(ctx, &barcode), &label) {
            Ok(x) => x,
            Err(e) => return err(500, code::INTERNAL, &e),
        };
        let cr = Cartridge {
            id: barcode.clone(),
            barcode,
            pool_id: r.pool_id.clone(),
            pool_name: pool.as_ref().map_or(String::new(), |p| p.name.clone()),
            capacity,
            used: tape.end,
            state: String::from("blank"),
            created: t,
            ..Cartridge::default()
        };
        put_cart(ctx, &cr);
        made.push(cr);
    }
    if made.is_empty() {
        return field(code::CONFLICT, "Mit diesem Präfix sind keine Barcodes mehr frei", "prefix");
    }
    c.audit("tape.add", &format!("{} Bänder", made.len()), &format!("{} … {}, je {}", made[0].barcode, made[made.len() - 1].barcode, size(capacity)));
    list(&made)
}

/// a blank tape to a pool, or free
pub fn tape_update(ctx: &Ctx, c: &Call) -> Response {
    let Some(r) = CartridgeUpdate::from_json(&c.body) else { return util::bad("Ungültige Anfrage") };
    let Some(mut cr) = cart(ctx, &c.param("id")) else { return not_found("Band nicht gefunden") };
    if cr.state != "blank" {
        return err(409, code::CONFLICT, "Nur leere Bänder wechseln den Pool");
    }
    let pool = if r.pool_id.is_empty() { None } else { pool_of(ctx, &r.pool_id) };
    if !r.pool_id.is_empty() && pool.is_none() {
        return field(code::BAD_REQUEST, "Medienpool nicht gefunden", "pool_id");
    }
    cr.pool_id = r.pool_id;
    cr.pool_name = pool.map_or(String::new(), |p| p.name);
    put_cart(ctx, &cr);
    c.audit("tape.update", &cr.barcode, if cr.pool_name.is_empty() { "frei" } else { &cr.pool_name });
    ok(200, &cr)
}

fn protected(cr: &Cartridge) -> bool {
    cr.protected_until > now()
}

pub fn tape_delete(ctx: &Ctx, c: &Call) -> Response {
    let Some(cr) = cart(ctx, &c.param("id")) else { return not_found("Band nicht gefunden") };
    if protected(&cr) {
        return err(409, code::IMMUTABLE, &format!("Das Band ist bis {} schreibgeschützt", crate::notify::when(cr.protected_until)));
    }
    let _d = DRIVE.lock();
    plat::delete(&tape_path(ctx, &cr.barcode));
    let gone = drop_sets_on(ctx, &cr.barcode);
    let _ = ctx.db.delete(C_TAPES, &cr.id);
    c.audit("tape.remove", &cr.barcode, &format!("{} Sätze entfernt", gone));
    no_content()
}

pub fn start_erase(ctx: &Arc<Ctx>, c: &Call) -> Response {
    let Some(cr) = cart(ctx, &c.param("id")) else { return not_found("Band nicht gefunden") };
    if protected(&cr) {
        return err(409, code::IMMUTABLE, &format!("Das Band ist bis {} schreibgeschützt", crate::notify::when(cr.protected_until)));
    }
    if !present(ctx, &cr.barcode) {
        return err(409, code::CONFLICT, "Das Band ist nicht in der Bibliothek");
    }
    let label = cr.barcode.clone();
    let op = Ops::start(ctx, "tape.erase", &format!("Band {} löschen", cr.barcode), &c.actor(), &cr.id.clone(), "", Box::new(move |o| {
        let ctx = o.ctx().clone();
        let _d = DRIVE.lock();
        let mut t = bt::Tape::open(&tape_path(&ctx, &cr.barcode), true)?;
        t.erase()?;
        let gone = drop_sets_on(&ctx, &cr.barcode);
        let mut x = cart(&ctx, &cr.id).unwrap_or(cr.clone());
        x.used = t.end;
        x.state = String::from("blank");
        x.sets = 0;
        x.written = 0;
        x.protected_until = 0;
        x.message.clear();
        put_cart(&ctx, &x);
        Ok(format!("Band {} ist leer ({} Sätze entfernt)", cr.barcode, gone))
    }));
    c.audit("tape.erase", &label, "");
    util::accepted(&op)
}

// ------------------------------------------------------------------ pools

pub fn pools(ctx: &Ctx) -> Response {
    let tapes: Vec<Cartridge> = all(&ctx.db, C_TAPES);
    let mut v: Vec<MediaPool> = all(&ctx.db, C_POOLS);
    for p in &mut v {
        let mine: Vec<&Cartridge> = tapes.iter().filter(|c| c.pool_id == p.id).collect();
        p.tapes = mine.len() as u32;
        p.used = mine.iter().map(|c| c.used).sum();
        p.capacity = mine.iter().map(|c| c.capacity).sum();
    }
    v.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    list(&v)
}

pub fn pool_save(ctx: &Ctx, c: &Call) -> Response {
    let Some(r) = PoolRequest::from_json(&c.body) else { return util::bad("Ungültige Anfrage") };
    let id = c.param("id");
    let old = if id.is_empty() { None } else { pool_of(ctx, &id) };
    if !id.is_empty() && old.is_none() {
        return not_found("Medienpool nicht gefunden");
    }
    let name = r.name.trim();
    if name.is_empty() || name.chars().count() > 80 {
        return field(code::BAD_REQUEST, "Bitte einen Namen angeben", "name");
    }
    if all::<MediaPool>(&ctx.db, C_POOLS).iter().any(|p| p.id != id && p.name.eq_ignore_ascii_case(name)) {
        return field(code::CONFLICT, "Diesen Namen hat schon ein Pool", "name");
    }
    if r.protect_days > 3650 {
        return field(code::BAD_REQUEST, "Höchstens 3650 Tage", "protect_days");
    }
    let p = MediaPool {
        id: if id.is_empty() { new_id() } else { id.clone() },
        name: String::from(name),
        description: r.description,
        protect_days: r.protect_days,
        encrypted: r.encrypted,
        append: r.append,
        created: old.as_ref().map_or(now(), |o| o.created),
        ..MediaPool::default()
    };
    let _ = ctx.db.put(C_POOLS, &p.id, p.to_json());
    if old.as_ref().is_some_and(|o| o.name != p.name) {
        for mut cr in all::<Cartridge>(&ctx.db, C_TAPES).into_iter().filter(|x| x.pool_id == p.id) {
            cr.pool_name = p.name.clone();
            put_cart(ctx, &cr);
        }
    }
    c.audit(if id.is_empty() { "pool.create" } else { "pool.update" }, &p.name, &format!("Schutz {} Tage{}", p.protect_days, if p.encrypted { ", verschlüsselt" } else { "" }));
    ok(if id.is_empty() { 201 } else { 200 }, &p)
}

pub fn pool_delete(ctx: &Ctx, c: &Call) -> Response {
    let id = c.param("id");
    let Some(p) = pool_of(ctx, &id) else { return not_found("Medienpool nicht gefunden") };
    let tapes: Vec<Cartridge> = all::<Cartridge>(&ctx.db, C_TAPES).into_iter().filter(|x| x.pool_id == id).collect();
    if tapes.iter().any(|x| x.state != "blank") {
        return err(409, code::CONFLICT, "Der Pool hat beschriebene Bänder");
    }
    if all::<Job>(&ctx.db, C_JOBS).iter().any(|j| j.kind == JobKind::Tape && j.pool_id == id) {
        return err(409, code::CONFLICT, "Ein Bandjob schreibt in diesen Pool");
    }
    for mut cr in tapes {
        cr.pool_id.clear();
        cr.pool_name.clear();
        put_cart(ctx, &cr);
    }
    let _ = ctx.db.delete(C_POOLS, &id);
    c.audit("pool.delete", &p.name, "");
    no_content()
}

// ------------------------------------------------------------------ writing a set

/// the tape the next part goes on: an appending pool's newest tape with room, a blank tape of
/// the pool, a free blank tape (it joins the pool)
fn take_tape(ctx: &Ctx, pool: &MediaPool, skip: &[String], append: bool) -> Result<(String, bt::Tape), String> {
    let tapes: Vec<Cartridge> = all::<Cartridge>(&ctx.db, C_TAPES).into_iter().filter(|c| !skip.contains(&c.barcode) && present(ctx, &c.barcode)).collect();
    let mut order: Vec<&Cartridge> = Vec::new();
    if append && pool.append {
        let mut act: Vec<&Cartridge> = tapes.iter().filter(|c| c.pool_id == pool.id && c.state == "active" && c.capacity.saturating_sub(c.used) >= MIN_ROOM).collect();
        act.sort_by_key(|c| core::cmp::Reverse(c.written));
        order.extend(act);
    }
    let mut blank: Vec<&Cartridge> = tapes.iter().filter(|c| c.pool_id == pool.id && c.state == "blank").collect();
    blank.sort_by(|a, b| a.barcode.cmp(&b.barcode));
    order.extend(blank);
    let mut free: Vec<&Cartridge> = tapes.iter().filter(|c| c.pool_id.is_empty() && c.state == "blank").collect();
    free.sort_by(|a, b| a.barcode.cmp(&b.barcode));
    order.extend(free);
    for c in order {
        match bt::Tape::open(&tape_path(ctx, &c.barcode), true) {
            Ok(t) if t.room() >= MIN_ROOM && t.label.barcode == c.barcode => {
                if c.pool_id != pool.id {
                    let mut x = c.clone();
                    x.pool_id = pool.id.clone();
                    x.pool_name = pool.name.clone();
                    put_cart(ctx, &x);
                }
                return Ok((c.barcode.clone(), t));
            }
            Ok(_) => {}
            Err(e) => {
                let mut x = c.clone();
                x.message = e;
                put_cart(ctx, &x);
            }
        }
    }
    Err(format!("Kein Band mit Platz im Pool „{}“ und keines frei – bitte Bänder hinzufügen", pool.name))
}

struct SetWriter<'a> {
    ctx: &'a Ctx,
    o: &'a OpCtl,
    pool: MediaPool,
    head: SetBegin,
    key: Option<SetKey>,
    tape: bt::Tape,
    barcode: String,
    tapes: Vec<String>,
    positions: Vec<u64>,
    /// the next block number
    n: u64,
    part: u32,
    entries: u64,
    files: u64,
    bytes: u64,
    total: u64,
    catalog: Vec<bt::Entry>,
}

impl SetWriter<'_> {
    fn seal(&self, kind: u8, plain: &[u8]) -> (Vec<u8>, u8) {
        match &self.key {
            Some(k) => (k.seal(self.n, kind, plain), bt::F_SEALED),
            None => (Vec::from(plain), 0),
        }
    }

    fn put(&mut self, kind: u8, plain: &[u8]) -> Result<(), String> {
        let (payload, flags) = self.seal(kind, plain);
        if self.tape.room() < (bt::HEADER + payload.len()) as u64 + RESERVE {
            self.next_tape()?;
            let (payload, flags) = self.seal(kind, plain);
            self.tape.append(kind, flags, &payload)?;
        } else {
            self.tape.append(kind, flags, &payload)?;
        }
        self.n += 1;
        Ok(())
    }

    fn begin_part(&mut self) -> Result<(), String> {
        let mut h = self.head.clone();
        h.part = self.part;
        h.block0 = self.n;
        h.created = now();
        let pos = self.tape.append(bt::K_SET_BEGIN, 0, j::text(&h.to_json()).as_bytes())?;
        self.tapes.push(self.barcode.clone());
        self.positions.push(pos);
        Ok(())
    }

    fn end_part(&mut self, last: bool, aborted: bool) -> Result<(), String> {
        let e = SetEnd { set_id: self.head.set_id.clone(), part: self.part, last, aborted, entries: self.entries, files: self.files, bytes: self.bytes, blocks: self.n + 1 };
        let (payload, flags) = self.seal(bt::K_SET_END, j::text(&e.to_json()).as_bytes());
        self.tape.append(bt::K_SET_END, flags, &payload)?;
        self.n += 1;
        self.tape.sync();
        self.note_tape();
        Ok(())
    }

    /// the record of the current tape after writing
    fn note_tape(&self) {
        let t = now();
        let mut c = cart(self.ctx, &self.barcode).unwrap_or_default();
        c.used = self.tape.end;
        c.written = t;
        c.state = String::from(if self.tape.room() < MIN_ROOM { "full" } else { "active" });
        c.pool_id = self.pool.id.clone();
        c.pool_name = self.pool.name.clone();
        c.protected_until = c.protected_until.max(t + self.pool.protect_days as i64 * DAY);
        c.message.clear();
        put_cart(self.ctx, &c);
    }

    fn next_tape(&mut self) -> Result<(), String> {
        self.end_part(false, false)?;
        let (barcode, tape) = take_tape(self.ctx, &self.pool, &self.tapes, false)?;
        self.barcode = barcode;
        self.tape = tape;
        self.part += 1;
        self.begin_part()
    }

    fn walk(&mut self, repo: &Repo, n: &Node, r: u32, path: &str, s: &mut Session) -> Result<(), String> {
        if self.o.cancelled() {
            return Err(String::from("Abgebrochen"));
        }
        let i = self.entries;
        self.entries += 1;
        let e = bt::Entry {
            i,
            r,
            p: String::from(path),
            k: String::from(match n.kind {
                NodeKind::Dir => "dir",
                NodeKind::File => "file",
                NodeKind::Symlink => "link",
                NodeKind::Other => "other",
            }),
            size: n.size,
            mtime: n.mtime_ns,
            atime: n.atime_ns,
            mode: n.mode,
            attrs: n.attrs,
            link: n.link.clone(),
        };
        self.put(bt::K_ENTRY, j::text(&e.to_json()).as_bytes())?;
        self.catalog.push(e);
        match n.kind {
            NodeKind::File => {
                let mut h = crypto::blake3::Blake3::new();
                let mut off = 0u64;
                let mut damaged = false;
                for id in &n.chunks {
                    let data = match repo.read_chunk(id) {
                        Ok(d) => d,
                        Err(x) => {
                            s.errors += 1;
                            engine::session_log(self.ctx, s, LogLevel::Error, &format!("{}: {}", path, engine::err_text(x)));
                            damaged = true;
                            break;
                        }
                    };
                    for piece in data.chunks(bt::DATA_MAX) {
                        self.put(bt::K_DATA, &bt::data_payload(i, off, piece))?;
                        h.update(piece);
                        off += piece.len() as u64;
                        self.bytes += piece.len() as u64;
                    }
                    self.o.progress(self.bytes, self.total.max(self.bytes));
                }
                let hash = if damaged { String::new() } else { crypto::hex(&h.finalize()) };
                self.put(bt::K_FILE_END, j::text(&bt::FileEnd { i, hash }.to_json()).as_bytes())?;
                self.files += 1;
                s.files_done = self.files;
                s.bytes_read = self.bytes;
                if self.files % 500 == 0 {
                    engine::save(self.ctx, s);
                }
            }
            NodeKind::Dir => {
                if let Some(t) = n.tree {
                    let tree = repo.read_tree(&t).map_err(engine::err_text)?;
                    for c in &tree.nodes {
                        let child = if path.is_empty() { c.name.clone() } else { format!("{}/{}", path, c.name) };
                        self.walk(repo, c, r, &child, s)?;
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }
}

fn host_name(ctx: &Ctx, job: &Job, p: &RestorePoint) -> String {
    if !p.host.is_empty() {
        return p.host.clone();
    }
    rec::<Host>(ctx.get(C_HOSTS, &job.host_id)).map_or(String::new(), |h| h.name)
}

/// a restore point onto tapes of a pool
fn write_set(o: &OpCtl, ctx: &Ctx, p: &RestorePoint, pool: &MediaPool, job: &Job, s: &mut Session) -> Result<String, String> {
    let _d = DRIVE.lock();
    let (_, repo) = engine::repo_for(ctx, &p.repository_id)?;
    let snap = repo.load_snapshot(&p.id).map_err(engine::err_text)?;
    let set_id = new_id();
    // the set belongs to the point's job (a tape job only writes it)
    let src_job = job_or(ctx, &p.job_id, &p.job_name);
    let _ = job;
    let windows = snap.roots.iter().any(|r| r.name.as_bytes().get(1) == Some(&b':') || r.name.starts_with("\\\\"));
    let key = pool.encrypted.then(|| (plat::random_bytes::<32>(), plat::random_bytes::<16>()));
    let recovery = match (&key, ctx.keys.recovery_public()) {
        (Some((k, _)), Some(pk)) => crypto::sealbox::seal(&pk, &plat::random_bytes::<56>(), bt::AAD_RECOVERY, k).map_or(String::new(), |b| crypto::b64::encode_url(&b)),
        _ => String::new(),
    };
    let head = SetBegin {
        set_id: set_id.clone(),
        part: 1,
        pool_id: pool.id.clone(),
        pool_name: pool.name.clone(),
        job_id: p.job_id.clone(),
        job_name: p.job_name.clone(),
        point_id: p.id.clone(),
        point_time: p.created,
        host_name: host_name(ctx, &src_job, p),
        windows,
        roots: snap.roots.iter().map(|r| r.name.clone()).collect(),
        encrypted: key.is_some(),
        nonce: key.as_ref().map_or(String::new(), |(_, n)| crypto::hex(n)),
        recovery,
        key_check: key.as_ref().map_or(String::new(), |(k, _)| bt::key_check(k)),
        block0: 0,
        created: now(),
    };
    let (barcode, tape) = take_tape(ctx, pool, &[], true)?;
    engine::session_log(ctx, s, LogLevel::Info, &format!("Satz {} für Restore-Point vom {} beginnt auf Band {}", &set_id[..8], crate::notify::when(p.created), barcode));
    let mut w = SetWriter {
        ctx,
        o,
        pool: pool.clone(),
        head,
        key: key.as_ref().map(|(k, n)| SetKey::new(*k, *n, &set_id)),
        tape,
        barcode,
        tapes: Vec::new(),
        positions: Vec::new(),
        n: 0,
        part: 1,
        entries: 0,
        files: 0,
        bytes: 0,
        total: p.size,
        catalog: Vec::new(),
    };
    s.bytes_total = p.size;
    s.files_total = p.files;
    w.begin_part()?;
    let mut res = Ok(());
    for (ri, root) in snap.roots.iter().enumerate() {
        if let Err(e) = w.walk(&repo, root, ri as u32, "", s) {
            res = Err(e);
            break;
        }
    }
    if let Err(e) = res {
        let _ = w.end_part(true, true);
        return Err(format!("{} (der begonnene Satz auf {} ist unvollständig)", e, w.tapes.join(", ")));
    }
    w.end_part(true, false)?;
    let set = TapeSet {
        id: set_id.clone(),
        pool_id: pool.id.clone(),
        pool_name: pool.name.clone(),
        job_id: p.job_id.clone(),
        job_name: p.job_name.clone(),
        point_id: p.id.clone(),
        point_time: p.created,
        host_name: w.head.host_name.clone(),
        windows,
        roots: w.head.roots.clone(),
        tapes: w.tapes.clone(),
        positions: w.positions.clone(),
        files: w.files,
        entries: w.entries,
        bytes: w.bytes,
        encrypted: key.is_some(),
        written: now(),
        verified: 0,
        status: String::from("ok"),
    };
    put_set(ctx, &set);
    if let Some((k, _)) = &key {
        store_key(ctx, &set_id, k);
    }
    for b in &w.tapes {
        if let Some(mut c) = cart(ctx, b) {
            c.sets += 1;
            put_cart(ctx, &c);
        }
    }
    save_catalog(ctx, &set_id, &w.catalog);
    s.bytes_read = w.bytes;
    Ok(format!("{} Dateien ({}) auf {} geschrieben", w.files, size(w.bytes), w.tapes.join(", ")))
}

fn save_catalog(ctx: &Ctx, set: &str, entries: &[bt::Entry]) {
    let _ = plat::fs::mkdir_private(&ctx.path("tape"));
    let v = Value::Array(entries.iter().map(|e| e.to_json()).collect());
    let _ = plat::fs::write_file_private(&catalog_path(ctx, set), j::text(&v).as_bytes());
}

fn load_catalog(ctx: &Ctx, set: &str) -> Option<Vec<bt::Entry>> {
    let b = plat::read_file(&catalog_path(ctx, set))?;
    let v = j::parse(&b)?;
    Some(v.as_array()?.iter().filter_map(bt::Entry::from_json).collect())
}

fn job_or(ctx: &Ctx, id: &str, name: &str) -> Job {
    rec::<Job>(ctx.get(C_JOBS, id)).unwrap_or(Job { id: String::from(id), name: String::from(name), ..Job::default() })
}

/// POST /api/points/{id}/tape
pub fn point_write(ctx: &Arc<Ctx>, c: &Call) -> Response {
    let Some(p): Option<RestorePoint> = rec(ctx.get(C_POINTS, &c.param("id"))) else { return not_found("Restore-Point nicht gefunden") };
    let Some(r) = TapeWriteRequest::from_json(&c.body) else { return util::bad("Ungültige Anfrage") };
    let Some(pool) = pool_of(ctx, &r.pool_id) else { return field(code::BAD_REQUEST, "Bitte einen Medienpool wählen", "pool_id") };
    let job = job_or(ctx, &p.job_id, &p.job_name);
    let mut session = engine::session_for(ctx, SessionKind::Tape, &job, &p.id);
    let sid = session.id.clone();
    let pid = p.id.clone();
    let op = Ops::start(ctx, "tape.write", &format!("{}: auf Band ({})", p.job_name, pool.name), &c.actor(), &pid, &sid, Box::new(move |o| {
        let ctx = o.ctx().clone();
        engine::session_log(&ctx, &session, LogLevel::Info, &format!("Auf Band schreiben: Pool {}", pool.name));
        let res = write_set(o, &ctx, &p, &pool, &job, &mut session);
        engine::finish_session(&ctx, &mut session, &res, o.cancelled());
        res
    }));
    c.audit("tape.write", &c.param("id"), "");
    util::accepted(&op)
}

/// a tape job: the newest point of the source job that the pool lacks
pub fn start_job(ctx: &Arc<Ctx>, job: &Job, actor: &str) -> Result<String, String> {
    let pool = pool_of(ctx, &job.pool_id).ok_or("Der Medienpool des Bandjobs fehlt")?;
    let mut session = engine::session_for(ctx, SessionKind::Tape, job, "");
    engine::set_status(ctx, &job.id, SessionStatus::Running, None);
    let job = job.clone();
    let sid = session.id.clone();
    let op = Ops::start(ctx, "job.run", &format!("{}: auf Band", job.name), actor, &job.id.clone(), &sid, Box::new(move |o| {
        let ctx = o.ctx().clone();
        let have: BTreeSet<String> = all::<TapeSet>(&ctx.db, C_TAPESETS).into_iter().filter(|s| s.pool_id == pool.id && s.status == "ok").map(|s| s.point_id).collect();
        let newest = all::<RestorePoint>(&ctx.db, C_POINTS).into_iter().filter(|p| p.job_id == job.source_job).max_by_key(|p| p.order());
        let res = match newest {
            None => Err(String::from("Der Quelljob hat noch keinen Restore-Point")),
            Some(p) if have.contains(&p.id) => Ok(String::from("Der neueste Restore-Point ist schon auf Band")),
            Some(p) => {
                session.point_id = p.id.clone();
                write_set(o, &ctx, &p, &pool, &job, &mut session)
            }
        };
        engine::finish_session(&ctx, &mut session, &res, o.cancelled());
        engine::set_status(&ctx, &job.id, session.status, Some(&session));
        res
    }));
    let _ = ctx.db.update(crate::ctx::C_SESSIONS, &sid, |v| {
        v.insert("operation_id", Value::string(op.as_str()));
    });
    Ok(op)
}

/// tape jobs without a schedule run after each good backup of their source job
pub fn after_backup(ctx: &Ctx, source: &Job) {
    let Some(arc) = crate::ctx::current() else { return };
    for j in all::<Job>(&ctx.db, C_JOBS) {
        if j.kind == JobKind::Tape && j.enabled && j.source_job == source.id && j.schedule.cron.trim().is_empty() && !ctx.ops.busy(&j.id, "job.run") {
            let _ = start_job(&arc, &j, "Nach Quelljob");
        }
    }
}

// ------------------------------------------------------------------ sets: list, browse

pub fn sets(ctx: &Ctx, c: &Call) -> Response {
    let job = c.query("job").unwrap_or_default();
    let mut v: Vec<TapeSet> = all::<TapeSet>(&ctx.db, C_TAPESETS).into_iter().filter(|s| job.is_empty() || s.job_id == job).collect();
    for s in &mut v {
        if s.status == "ok" && s.tapes.iter().any(|t| !present(ctx, t)) {
            s.status = String::from("offline");
        }
    }
    v.sort_by_key(|s| core::cmp::Reverse(s.written));
    list(&v)
}

fn norm(p: &str, win: bool) -> String {
    let s = String::from(p.replace('\\', "/").trim_end_matches('/'));
    if win { s.to_lowercase() } else { s }
}

pub fn set_browse(ctx: &Ctx, c: &Call) -> Response {
    let Some(s) = set_of(ctx, &c.param("id")) else { return not_found("Bandsatz nicht gefunden") };
    let path = c.query("path").unwrap_or_default();
    let offset = c.query("offset").and_then(|o| o.parse().ok()).unwrap_or(0usize);
    let Some(cat) = load_catalog(ctx, &s.id) else { return err(409, code::CONFLICT, "Für diesen Satz gibt es keinen Katalog (Bibliothek neu einlesen)") };
    let kind = |k: &str| match k {
        "dir" => EntryKind::Dir,
        "file" => EntryKind::File,
        "link" => EntryKind::Link,
        _ => EntryKind::Other,
    };
    let name_of = |e: &bt::Entry| -> String {
        if e.p.is_empty() { s.roots.get(e.r as usize).cloned().unwrap_or_default() } else { String::from(e.p.rsplit('/').next().unwrap_or(&e.p)) }
    };
    let to_entry = |e: &bt::Entry| Entry { name: name_of(e), kind: kind(&e.k), size: e.size, mtime: e.mtime.div_euclid(1_000_000_000) };
    let mut out: Vec<Entry>;
    if path.is_empty() {
        out = cat.iter().filter(|e| e.p.is_empty()).map(to_entry).collect();
    } else {
        let want = norm(&path, s.windows);
        // the folder: a root, or below one
        let Some(dir) = cat.iter().find(|e| norm(&bt::full_path(&s.roots, e, s.windows), s.windows) == want) else {
            return err(409, code::CONFLICT, &format!("„{}“ nicht im Bandsatz", path));
        };
        if dir.k != "dir" {
            return err(409, code::CONFLICT, "Das ist kein Ordner");
        }
        let prefix = if dir.p.is_empty() { String::new() } else { format!("{}/", dir.p) };
        out = cat.iter().filter(|e| e.r == dir.r && !e.p.is_empty() && e.p.starts_with(&prefix) && !e.p[prefix.len()..].contains('/')).map(to_entry).collect();
    }
    out.sort_by(|a, b| ((a.kind != EntryKind::Dir), a.name.to_lowercase()).cmp(&((b.kind != EntryKind::Dir), b.name.to_lowercase())));
    let more = out.len() > offset + 1000;
    ok(200, &Listing { path, entries: out.into_iter().skip(offset).take(1000).collect(), more, offset: offset as u32 })
}

// ------------------------------------------------------------------ reading a set

/// where a restore from tape writes
enum Sink<'a> {
    Local { root: Option<String> },
    Agent(&'a Host),
}

/// a file being restored
struct Out {
    i: u64,
    e: bt::Entry,
    dest: String,
    tmp: String,
    file: Option<plat::File>,
    hash: crypto::blake3::Blake3,
    bad: bool,
}

struct Reader<'a> {
    ctx: &'a Ctx,
    o: &'a OpCtl,
    set: &'a TapeSet,
    key: Option<SetKey>,
    done: u64,
}

/// what the reading found
#[derive(Default)]
struct ReadStats {
    entries: u64,
    files: u64,
    bytes: u64,
    bad: u64,
    complete: bool,
}

impl Reader<'_> {
    /// every block of the set in order: f(kind, plain payload)
    fn each(&mut self, f: &mut dyn FnMut(&mut Self, u8, &[u8]) -> Result<(), String>) -> Result<bool, String> {
        for (k, barcode) in self.set.tapes.iter().enumerate() {
            let tape = bt::Tape::open(&tape_path(self.ctx, barcode), false).map_err(|e| format!("Band {}: {}", barcode, e))?;
            let pos = *self.set.positions.get(k).ok_or("Satz ohne Position")?;
            let b = tape.read(pos)?.ok_or_else(|| format!("Band {}: der Satz fehlt", barcode))?;
            let head: SetBegin = b.json().filter(|h: &SetBegin| b.kind == bt::K_SET_BEGIN && h.set_id == self.set.id && h.part as usize == k + 1)
                .ok_or_else(|| format!("Band {}: anderer Inhalt an der Stelle des Satzes (gelöscht?)", barcode))?;
            if self.set.encrypted && self.key.is_none() {
                return Err(String::from("Der Schlüssel des Satzes fehlt"));
            }
            let mut n = head.block0;
            let mut at = b.next;
            loop {
                if self.o.cancelled() {
                    return Err(String::from("Abgebrochen"));
                }
                let Some(b) = tape.read(at)? else { return Err(format!("Band {}: der Satz endet unerwartet", barcode)) };
                at = b.next;
                let plain = match (&self.key, b.sealed) {
                    (Some(key), true) => key.open(n, b.kind, &b.payload).ok_or_else(|| format!("Band {}: Block {} lässt sich nicht entschlüsseln", barcode, n))?,
                    (None, false) => b.payload,
                    _ => return Err(format!("Band {}: Block {} passt nicht zur Verschlüsselung des Satzes", barcode, n)),
                };
                n += 1;
                if b.kind == bt::K_SET_END {
                    let e: SetEnd = j::parse(&plain).and_then(|v| SetEnd::from_json(&v)).ok_or("unlesbares Satzende")?;
                    f(self, b.kind, &plain)?;
                    if e.last {
                        return Ok(!e.aborted);
                    }
                    break;
                }
                f(self, b.kind, &plain)?;
            }
        }
        Ok(false)
    }
}

fn selected(set: &TapeSet, e: &bt::Entry, items: &[String]) -> bool {
    if items.is_empty() {
        return true;
    }
    let full = norm(&bt::full_path(&set.roots, e, set.windows), set.windows);
    items.iter().any(|it| {
        let x = norm(it, set.windows);
        full == x || full.starts_with(&format!("{}/", x))
    })
}

fn open_key(ctx: &Ctx, set: &TapeSet) -> Result<Option<SetKey>, String> {
    if !set.encrypted {
        return Ok(None);
    }
    let key = key_of(ctx, &set.id).ok_or("Der Schlüssel des Satzes ist auf diesem Server nicht vorhanden")?;
    let barcode = set.tapes.first().ok_or("Satz ohne Band")?;
    let tape = bt::Tape::open(&tape_path(ctx, barcode), false)?;
    let b = tape.read(*set.positions.first().unwrap_or(&0))?.ok_or("Der Satz fehlt auf dem Band")?;
    let head: SetBegin = b.json().ok_or("unlesbarer Satzanfang")?;
    SetKey::for_set(key, &head).map(Some).ok_or_else(|| String::from("Der Schlüssel passt nicht zum Satz"))
}

pub fn start_set_verify(ctx: &Arc<Ctx>, c: &Call) -> Response {
    let Some(set) = set_of(ctx, &c.param("id")) else { return not_found("Bandsatz nicht gefunden") };
    let job = job_or(ctx, &set.job_id, &set.job_name);
    let mut session = engine::session_for(ctx, SessionKind::Verify, &job, &set.point_id);
    let sid = session.id.clone();
    let op = Ops::start(ctx, "tape.verify", &format!("{}: Bandsatz prüfen", set.job_name), &c.actor(), &set.id.clone(), &sid, Box::new(move |o| {
        let ctx = o.ctx().clone();
        let res = verify_set(o, &ctx, &set, &mut session);
        engine::finish_session(&ctx, &mut session, &res, o.cancelled());
        if res.is_ok() {
            if let Some(mut x) = set_of(&ctx, &set.id) {
                x.verified = now();
                put_set(&ctx, &x);
            }
        }
        res
    }));
    c.audit("tape.verify", &c.param("id"), "");
    util::accepted(&op)
}

fn verify_set(o: &OpCtl, ctx: &Ctx, set: &TapeSet, s: &mut Session) -> Result<String, String> {
    let _d = DRIVE.lock();
    let key = open_key(ctx, set)?;
    let mut r = Reader { ctx, o, set, key, done: 0 };
    let mut st = ReadStats::default();
    let mut cur: Option<(u64, crypto::blake3::Blake3)> = None;
    s.bytes_total = set.bytes;
    s.files_total = set.files;
    let total = set.bytes;
    let complete = r.each(&mut |rd, kind, p| {
        match kind {
            bt::K_ENTRY => {
                let e = j::parse(p).and_then(|v| bt::Entry::from_json(&v)).ok_or("unlesbarer Eintrag")?;
                st.entries += 1;
                cur = (e.k == "file").then(|| (e.i, crypto::blake3::Blake3::new()));
            }
            bt::K_DATA => {
                let (i, _, bytes) = bt::parse_data(p).ok_or("unlesbarer Datenblock")?;
                if let Some((ci, h)) = &mut cur {
                    if *ci == i {
                        h.update(bytes);
                    }
                }
                st.bytes += bytes.len() as u64;
                rd.done += bytes.len() as u64;
                rd.o.progress(rd.done, total.max(rd.done));
            }
            bt::K_FILE_END => {
                let fe = j::parse(p).and_then(|v| bt::FileEnd::from_json(&v)).ok_or("unlesbares Dateiende")?;
                st.files += 1;
                if let Some((ci, h)) = cur.take() {
                    if ci == fe.i && (fe.hash.is_empty() || crypto::hex(&h.finalize()) != fe.hash) {
                        st.bad += 1;
                    }
                }
            }
            bt::K_SET_END => {
                let e = j::parse(p).and_then(|v| SetEnd::from_json(&v)).ok_or("unlesbares Satzende")?;
                if e.last && (e.entries != st.entries || e.files != st.files || e.bytes != st.bytes) {
                    return Err(format!("Die Zählung stimmt nicht: {} / {} Einträge, {} / {} Dateien", st.entries, e.entries, st.files, e.files));
                }
            }
            _ => {}
        }
        Ok(())
    })?;
    st.complete = complete;
    s.files_done = st.files;
    s.bytes_read = st.bytes;
    if !st.complete {
        return Err(String::from("Der Satz ist unvollständig (abgebrochen geschrieben)"));
    }
    if st.bad > 0 {
        s.errors += st.bad as u32;
        return Err(format!("{} von {} Dateien weichen ab", st.bad, st.files));
    }
    Ok(format!("{} Dateien ({}) von {} gelesen und geprüft", st.files, size(st.bytes), set.tapes.join(", ")))
}

pub fn start_set_restore(ctx: &Arc<Ctx>, c: &Call) -> Response {
    let Some(set) = set_of(ctx, &c.param("id")) else { return not_found("Bandsatz nicht gefunden") };
    let Some(req) = RestoreRequest::from_json(&c.body) else { return util::bad("Ungültige Anfrage") };
    let role = c.user.as_ref().map_or(Role::Viewer, |u| u.role);
    let host = if req.host_id.is_empty() {
        all::<Host>(&ctx.db, C_HOSTS).into_iter().find(|h| h.kind == HostKind::Server)
    } else {
        rec::<Host>(ctx.get(C_HOSTS, &req.host_id))
    };
    let Some(host) = host else { return field(code::BAD_REQUEST, "Zielhost nicht gefunden", "host_id") };
    let win = crate::hosts::host_windows(&host);
    let operator = role.allows(Role::Operator);
    if req.original && !operator {
        return err(403, code::FORBIDDEN, "An den ursprünglichen Ort dürfen nur Operatoren wiederherstellen");
    }
    if req.conflict == Conflict::Overwrite && !operator {
        return err(403, code::FORBIDDEN, "Vorhandene Dateien ersetzen dürfen nur Operatoren");
    }
    let target = String::from(req.target.trim());
    let roots = crate::restore::allowed_roots(ctx, &host);
    if !req.original {
        if !absolute(&target, win) {
            return field(code::BAD_REQUEST, "Bitte einen absoluten Zielordner angeben", "target");
        }
        if !inside_os(&roots, &target, win) {
            return field(code::FORBIDDEN, "Dieser Zielordner ist für Wiederherstellungen nicht freigegeben", "target");
        }
    }
    let items: Vec<String> = req.items.iter().map(|i| String::from(i.trim())).filter(|i| !i.is_empty() && i != "/").collect();
    let job = job_or(ctx, &set.job_id, &set.job_name);
    let mut session = engine::session_for(ctx, SessionKind::Restore, &job, &set.point_id);
    let sid = session.id.clone();
    let conflict = String::from(req.conflict.as_str());
    let original = req.original;
    let root = if original { None } else { bc_fs::root_of(&roots, &target).map(String::from) };
    let what = (set.job_name.clone(), if items.is_empty() { String::from("alle Einträge") } else { format!("{} Einträge", items.len()) });
    let op = Ops::start(ctx, "tape.restore", &format!("{}: vom Band wiederherstellen", set.job_name), &c.actor(), &set.id.clone(), &sid, Box::new(move |o| {
        let ctx = o.ctx().clone();
        engine::session_log(&ctx, &session, LogLevel::Info, &format!("Vom Band ({}) nach {} ({})", set.tapes.join(", "), if original { "Originalort" } else { target.as_str() }, host.name));
        let sink = if host.kind == HostKind::Agent { Sink::Agent(&host) } else { Sink::Local { root: root.clone() } };
        let res = restore_set(o, &ctx, &set, &items, &sink, original, &target, win, &conflict, &mut session);
        engine::finish_session(&ctx, &mut session, &res, o.cancelled());
        res
    }));
    c.audit("tape.restore", &what.0, &what.1);
    util::accepted(&op)
}

/// the counts of a restore
#[derive(Default)]
struct Done {
    written: u64,
    skipped: u64,
    renamed: u64,
    warnings: u64,
}

#[allow(clippy::too_many_arguments)]
fn restore_set(o: &OpCtl, ctx: &Ctx, set: &TapeSet, items: &[String], sink: &Sink, original: bool, target: &str, win: bool, conflict: &str, s: &mut Session) -> Result<String, String> {
    if let Sink::Agent(h) = sink {
        crate::agents::info(ctx, h).map_err(|e| format!("Agent nicht erreichbar: {}", e))?;
    }
    let _d = DRIVE.lock();
    let key = open_key(ctx, set)?;
    let mut r = Reader { ctx, o, set, key, done: 0 };
    let mut out: Option<Out> = None;
    let mut dirs: Vec<(String, bt::Entry)> = Vec::new();
    let mut d = Done::default();
    let mut logs: Vec<(LogLevel, String)> = Vec::new();
    let total = set.bytes;
    let dest_of = |e: &bt::Entry| -> String {
        let full = bt::full_path(&set.roots, e, set.windows);
        if original { full } else { join_os(target, &crate::restore::relative_of(&full, win), win) }
    };
    let complete = r.each(&mut |rd, kind, p| {
        match kind {
            bt::K_ENTRY => {
                let e = j::parse(p).and_then(|v| bt::Entry::from_json(&v)).ok_or("unlesbarer Eintrag")?;
                if let Some(x) = out.take() {
                    sink_abort(rd.ctx, sink, x);
                }
                if !selected(set, &e, items) {
                    return Ok(());
                }
                if e.p.split('/').any(|n| !n.is_empty() && !bc_fs::safe_name(n, win)) {
                    logs.push((LogLevel::Warning, format!("unzulässiger Name übersprungen: {}", e.p)));
                    d.warnings += 1;
                    return Ok(());
                }
                let dest = dest_of(&e);
                match e.k.as_str() {
                    "dir" => match sink_dir(rd.ctx, sink, &dest, None) {
                        Ok(()) => dirs.push((dest, e)),
                        Err(x) => {
                            logs.push((LogLevel::Warning, format!("{}: {}", dest, x)));
                            d.warnings += 1;
                        }
                    },
                    "file" => match sink_begin(rd.ctx, sink, &dest, conflict, &e) {
                        Ok(Some((x, renamed))) => {
                            if renamed {
                                d.renamed += 1;
                            }
                            out = Some(x);
                        }
                        Ok(None) => d.skipped += 1,
                        Err(x) => {
                            logs.push((LogLevel::Warning, format!("{}: {}", dest, x)));
                            d.warnings += 1;
                        }
                    },
                    "link" => {
                        if let Err(x) = sink_link(rd.ctx, sink, &dest, &e, conflict) {
                            logs.push((LogLevel::Warning, format!("{}: {}", dest, x)));
                            d.warnings += 1;
                        }
                    }
                    _ => {
                        logs.push((LogLevel::Warning, format!("{}: Sonderdatei übersprungen", dest)));
                        d.warnings += 1;
                    }
                }
            }
            bt::K_DATA => {
                let (i, off, bytes) = bt::parse_data(p).ok_or("unlesbarer Datenblock")?;
                if let Some(x) = &mut out {
                    if x.i == i && !x.bad {
                        x.hash.update(bytes);
                        if let Err(e) = sink_write(rd.ctx, sink, x, off, bytes) {
                            logs.push((LogLevel::Warning, format!("{}: {}", x.dest, e)));
                            d.warnings += 1;
                            x.bad = true;
                        }
                    }
                }
                rd.done += bytes.len() as u64;
                rd.o.progress(rd.done, total.max(rd.done));
            }
            bt::K_FILE_END => {
                let fe = j::parse(p).and_then(|v| bt::FileEnd::from_json(&v)).ok_or("unlesbares Dateiende")?;
                if let Some(x) = out.take() {
                    if x.i != fe.i || x.bad {
                        sink_abort(rd.ctx, sink, x);
                    } else if fe.hash.is_empty() || crypto::hex(&x.hash.finalize()) != fe.hash {
                        logs.push((LogLevel::Error, format!("{}: die Daten auf dem Band weichen ab", x.dest)));
                        d.warnings += 1;
                        sink_abort(rd.ctx, sink, x);
                    } else {
                        let dest = x.dest.clone();
                        match sink_commit(rd.ctx, sink, x) {
                            Ok(()) => d.written += 1,
                            Err(e) => {
                                logs.push((LogLevel::Warning, format!("{}: {}", dest, e)));
                                d.warnings += 1;
                            }
                        }
                    }
                }
            }
            _ => {}
        }
        Ok(())
    });
    if let Some(x) = out.take() {
        sink_abort(ctx, sink, x);
    }
    for (lv, m) in &logs {
        if *lv == LogLevel::Error {
            s.errors += 1;
        } else {
            s.warnings += 1;
        }
        engine::session_log(ctx, s, *lv, m);
    }
    // the folders' times last (their files changed them)
    for (dest, e) in dirs.iter().rev() {
        let _ = sink_dir(ctx, sink, dest, Some(e));
    }
    s.files_done = d.written;
    s.bytes_read = r.done;
    let complete = complete?;
    if !complete {
        return Err(String::from("Der Satz ist unvollständig (abgebrochen geschrieben)"));
    }
    Ok(format!("{} Dateien vom Band wiederhergestellt, {} übersprungen, {} umbenannt", d.written, d.skipped, d.renamed))
}

// ------------------------------------------------------------------ restore targets

fn sink_dir(ctx: &Ctx, sink: &Sink, dest: &str, meta: Option<&bt::Entry>) -> Result<(), String> {
    match sink {
        Sink::Local { root } => {
            if let Some(e) = meta {
                bc_fs::apply_meta(dest, e.atime, e.mtime, e.mode, 0);
                return Ok(());
            }
            if let Some(r) = root {
                bc_fs::no_links(r, dest, false)?;
            }
            bc_fs::ensure_dir(dest)
        }
        Sink::Agent(h) => {
            let d = match meta {
                Some(e) => RestoreDir { path: String::from(dest), atime_ns: e.atime, mtime_ns: e.mtime, mode: e.mode },
                None => RestoreDir { path: String::from(dest), atime_ns: 0, mtime_ns: 0, mode: 0 },
            };
            crate::agents::restore_mkdir(ctx, h, &d)
        }
    }
}

/// a file to write (None: skipped because it exists); true when it got another name
fn sink_begin(ctx: &Ctx, sink: &Sink, dest: &str, conflict: &str, e: &bt::Entry) -> Result<Option<(Out, bool)>, String> {
    match sink {
        Sink::Local { root } => {
            if let Some(r) = root {
                bc_fs::no_links(r, dest, false)?;
            }
            let parent = plat::fs::parent(dest);
            if !parent.is_empty() {
                bc_fs::ensure_dir(parent)?;
            }
            let (t, p) = bc_fs::free_target(dest, conflict);
            let renamed = match t {
                Target::Skip => return Ok(None),
                Target::Renamed => true,
                Target::Free => false,
            };
            let tmp = bc_fs::temp_name(&p);
            let f = plat::File::create_new(&plat::fs::long_path(&tmp)).ok_or("kann nicht geschrieben werden")?;
            Ok(Some((Out { i: e.i, e: e.clone(), dest: p, tmp, file: Some(f), hash: crypto::blake3::Blake3::new(), bad: false }, renamed)))
        }
        Sink::Agent(h) => {
            let o = crate::agents::restore_open(ctx, h, dest, conflict)?;
            if o.skip {
                return Ok(None);
            }
            let renamed = o.dest != dest;
            Ok(Some((Out { i: e.i, e: e.clone(), dest: o.dest, tmp: o.tmp, file: None, hash: crypto::blake3::Blake3::new(), bad: false }, renamed)))
        }
    }
}

fn sink_write(ctx: &Ctx, sink: &Sink, x: &mut Out, off: u64, data: &[u8]) -> Result<(), String> {
    match sink {
        Sink::Local { .. } => {
            let f = x.file.as_ref().ok_or("keine Datei")?;
            if f.write_at(off, data) != Some(data.len()) {
                return Err(String::from("Schreibfehler (Speicher voll?)"));
            }
            Ok(())
        }
        Sink::Agent(h) => crate::agents::restore_write(ctx, h, &x.tmp, off, data),
    }
}

fn sink_commit(ctx: &Ctx, sink: &Sink, x: Out) -> Result<(), String> {
    match sink {
        Sink::Local { .. } => {
            let f = x.file.ok_or("keine Datei")?;
            bc_fs::finish_file(f, &x.tmp, &x.dest, x.e.atime, x.e.mtime, x.e.mode, x.e.attrs)
        }
        Sink::Agent(h) => {
            let c = RestoreCommit { tmp: x.tmp.clone(), dest: x.dest.clone(), atime_ns: x.e.atime, mtime_ns: x.e.mtime, mode: x.e.mode, attrs: x.e.attrs };
            crate::agents::restore_commit(ctx, h, &c).inspect_err(|_| crate::agents::restore_abort(ctx, h, &x.tmp))
        }
    }
}

fn sink_abort(ctx: &Ctx, sink: &Sink, x: Out) {
    match sink {
        Sink::Local { .. } => {
            drop(x.file);
            plat::delete(&plat::fs::long_path(&x.tmp));
        }
        Sink::Agent(h) => crate::agents::restore_abort(ctx, h, &x.tmp),
    }
}

fn sink_link(ctx: &Ctx, sink: &Sink, dest: &str, e: &bt::Entry, conflict: &str) -> Result<(), String> {
    let dir = e.attrs & bc_fs::ATTR_DIRECTORY != 0;
    match sink {
        Sink::Local { root } => {
            if let Some(r) = root {
                bc_fs::no_links(r, dest, false)?;
            }
            let parent = plat::fs::parent(dest);
            if !parent.is_empty() {
                bc_fs::ensure_dir(parent)?;
            }
            match bc_fs::free_target(dest, conflict) {
                (Target::Skip, _) => Ok(()),
                (_, p) => bc_fs::make_link(&e.link, &p, true, dir),
            }
        }
        Sink::Agent(h) => crate::agents::restore_symlink(ctx, h, &RestoreLink { path: String::from(dest), target: e.link.clone(), conflict: String::from(conflict), dir }),
    }
}

// ------------------------------------------------------------------ rescan

pub fn start_rescan(ctx: &Arc<Ctx>, c: &Call) -> Response {
    let op = Ops::start(ctx, "tape.rescan", "Bandbibliothek einlesen", &c.actor(), "", "", Box::new(move |o| {
        let ctx = o.ctx().clone();
        let _d = DRIVE.lock();
        rescan(o, &ctx)
    }));
    c.audit("tape.rescan", &library(ctx), "");
    util::accepted(&op)
}

/// a set seen while reading the library
struct Found {
    head: SetBegin,
    parts: BTreeMap<u32, (String, u64)>,
    last: bool,
    aborted: bool,
    end: Option<SetEnd>,
    catalog: Vec<bt::Entry>,
}

fn rescan(o: &OpCtl, ctx: &Ctx) -> Result<String, String> {
    let lib = library(ctx);
    let names: Vec<String> = plat::fs::read_dir(&lib).unwrap_or_default().into_iter().map(|e| e.name).filter(|n| n.ends_with(bt::EXT)).collect();
    let known: BTreeSet<String> = all::<TapeSet>(&ctx.db, C_TAPESETS).into_iter().map(|s| s.id).collect();
    let pools: BTreeMap<String, MediaPool> = all::<MediaPool>(&ctx.db, C_POOLS).into_iter().map(|p| (p.id.clone(), p)).collect();
    let mut found: BTreeMap<String, Found> = BTreeMap::new();
    let (mut seen, mut added, mut bad) = (0u32, 0u32, 0u32);
    for (k, name) in names.iter().enumerate() {
        o.progress(k as u64, names.len() as u64);
        if o.cancelled() {
            return Err(String::from("Abgebrochen"));
        }
        let tape = match bt::Tape::open(&plat::fs::join(&lib, name), false) {
            Ok(t) => t,
            Err(_) => {
                bad += 1;
                continue;
            }
        };
        let barcode = tape.label.barcode.clone();
        if format!("{}{}", barcode, bt::EXT) != *name {
            bad += 1;
            continue;
        }
        seen += 1;
        let mut pos = tape.read(0)?.map_or(0, |b| b.next);
        let label_end = pos;
        let mut cur: Option<(String, bool)> = None;
        let mut pool_id = String::new();
        let mut on: BTreeSet<String> = BTreeSet::new();
        let mut last_write = 0i64;
        while let Ok(Some(b)) = tape.read(pos) {
            pos = b.next;
            match b.kind {
                bt::K_SET_BEGIN => {
                    let Some(h) = b.json::<SetBegin>() else { continue };
                    on.insert(h.set_id.clone());
                    pool_id = h.pool_id.clone();
                    last_write = last_write.max(h.created);
                    let want = !known.contains(&h.set_id) && !h.encrypted;
                    let f = found.entry(h.set_id.clone()).or_insert_with(|| Found { head: h.clone(), parts: BTreeMap::new(), last: false, aborted: false, end: None, catalog: Vec::new() });
                    if h.part == 1 {
                        f.head = h.clone();
                    }
                    f.parts.insert(h.part, (barcode.clone(), b.pos));
                    cur = Some((h.set_id, want));
                }
                bt::K_ENTRY if !b.sealed => {
                    if let Some((id, true)) = &cur {
                        if let (Some(f), Some(e)) = (found.get_mut(id), b.json::<bt::Entry>()) {
                            f.catalog.push(e);
                        }
                    }
                }
                bt::K_SET_END => {
                    if let Some((id, _)) = cur.take() {
                        if let (Some(f), false) = (found.get_mut(&id), b.sealed) {
                            if let Some(e) = b.json::<SetEnd>() {
                                f.last |= e.last;
                                f.aborted |= e.aborted;
                                if e.last {
                                    f.end = Some(e);
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        let state = if tape.end <= label_end { "blank" } else if tape.room() < MIN_ROOM { "full" } else { "active" };
        let mut c = cart(ctx, &barcode).unwrap_or_else(|| {
            added += 1;
            Cartridge {
                id: barcode.clone(),
                barcode: barcode.clone(),
                pool_id: if pools.contains_key(&pool_id) { pool_id.clone() } else { String::new() },
                capacity: tape.label.capacity,
                created: tape.label.created,
                message: String::from("Eingelesen"),
                ..Cartridge::default()
            }
        });
        c.pool_name = pools.get(&c.pool_id).map_or(String::new(), |p| p.name.clone());
        c.used = tape.end;
        c.state = String::from(state);
        c.sets = on.len() as u32;
        c.written = c.written.max(last_write);
        put_cart(ctx, &c);
    }
    let mut cataloged = 0u32;
    for (id, f) in found {
        if known.contains(&id) || f.aborted {
            continue;
        }
        let parts: Vec<(u32, (String, u64))> = f.parts.into_iter().collect();
        let whole = parts.iter().enumerate().all(|(k, (p, _))| *p as usize == k + 1);
        let h = f.head;
        let end = f.end.unwrap_or_default();
        let set = TapeSet {
            id: id.clone(),
            pool_id: h.pool_id.clone(),
            pool_name: h.pool_name.clone(),
            job_id: h.job_id.clone(),
            job_name: h.job_name.clone(),
            point_id: h.point_id.clone(),
            point_time: h.point_time,
            host_name: h.host_name.clone(),
            windows: h.windows,
            roots: h.roots.clone(),
            tapes: parts.iter().map(|(_, (b, _))| b.clone()).collect(),
            positions: parts.iter().map(|(_, (_, p))| *p).collect(),
            files: end.files,
            entries: end.entries,
            bytes: end.bytes,
            encrypted: h.encrypted,
            written: h.created,
            verified: 0,
            status: String::from(if h.encrypted { "locked" } else if whole && f.last { "ok" } else { "incomplete" }),
        };
        put_set(ctx, &set);
        if !h.encrypted {
            save_catalog(ctx, &id, &f.catalog);
        }
        cataloged += 1;
    }
    let mut msg = format!("{} Bänder gelesen ({} neu), {} Sätze katalogisiert", seen, added, cataloged);
    if bad > 0 {
        msg.push_str(&format!(", {} Dateien sind keine lesbaren Bänder", bad));
    }
    Ok(msg)
}
