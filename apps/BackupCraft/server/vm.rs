// vm -- QEMU/KVM on the backup server: the VMs of its QMP folders, VM jobs (consistent disk
// backups by QEMU itself, incremental through dirty bitmaps), instant start (a VM boots from
// a restore point through a read-only NBD export of the repository; a qcow2 overlay takes its
// writes; "Zusammenführen" mirrors the disks into a folder while it runs) and boot checks (a VM
// of a point started without network access and checked)
//
//   status / list                       GET /api/hypervisors, /api/vms (rescanned after 10 s)
//   start_job(ctx, job, full, actor)    a VM job (JobKind::Qemu): job.sources are VM ids
//   instant_start / instant_list / instant_stop / instant_migrate / instant_console
//   bootcheck_start / bootcheck_list
//   start(ctx)                          at the start of the server: VMs of a former run are
//                                       gone (their sessions "interrupted")
//
// A VM in a point is a folder named like the VM: vm.json (memory, CPUs, the disks with node,
// file, size, bus) and one raw image per disk ("disk0.raw", ...) stored in blocks of
// bc_vm::BLOCK -- block i of a disk is chunk i of its file, so an incremental backup replaces
// only the dirty blocks and the NBD export reads any block at once.  A VM that does not run is
// read from its disk files (qcow2 through qemu-img convert); the next backup after such a cold
// one is full again.  Backups are crash-consistent (no guest agent).

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

use bc_proto::api::{code, BootCheckRequest, InstantRequest, MigrateRequest};
use bc_proto::j::{self, Json, Value};
use bc_proto::model::*;
use bc_vm::nbd::{self, Export, NbdServer};
use bc_vm::{Qmp, BLOCK};
use cas::{Node, NodeKind, Repo, Snapshot, Tree, KIND_DATA};
use netio::http::Response;
use plat::sync::Mutex;

use crate::api::Call;
use crate::ctx::{Ctx, C_BOOTCHECK, C_HOSTS, C_INSTANT, C_JOBS, C_POINTS, C_REPOS, C_VMS};
use crate::engine::{self, size};
use crate::ops::{OpCtl, Ops};
use crate::util::{self, all, err, field, list, new_id, not_found, now, ok, rec};

const DAY: i64 = 86_400;

// ------------------------------------------------------------------ settings

fn work(ctx: &Ctx) -> String {
    let s = ctx.setting("hypervisor.work");
    if s.trim().is_empty() { ctx.path("vm") } else { String::from(s.trim()) }
}

fn qemu(ctx: &Ctx) -> String {
    let s = ctx.setting("hypervisor.qemu");
    if s.trim().is_empty() { String::from("qemu-system-x86_64") } else { String::from(s.trim()) }
}

/// qemu-img next to the emulator (or from PATH)
fn qemu_img(ctx: &Ctx) -> String {
    let q = qemu(ctx);
    match q.rsplit_once('/') {
        Some((dir, _)) => format!("{}/qemu-img", dir),
        None => String::from("qemu-img"),
    }
}

fn folders(ctx: &Ctx) -> Vec<String> {
    ctx.setting("hypervisor.qmp").lines().map(|l| String::from(l.trim())).filter(|l| !l.is_empty()).collect()
}

/// hardware virtualization for this user
fn kvm() -> bool {
    cfg!(target_os = "linux") && plat::fs::lstat("/dev/kvm").is_some() && plat::File::open("/dev/kvm", plat::FO_READ).is_some()
}

fn server_host(ctx: &Ctx) -> Option<Host> {
    all::<Host>(&ctx.db, C_HOSTS).into_iter().find(|h| h.kind == HostKind::Server)
}

/// run a tool and wait for it (stderr in the error)
fn run_tool(program: &str, args: &[&str], timeout_s: u32) -> Result<(), String> {
    let mut c = plat::process::Command { program, args, cwd: None, env: &[] }.spawn().map_err(|e| format!("{} startet nicht (Fehler {})", program, e))?;
    let mut errtext = Vec::new();
    let start = plat::time_us();
    loop {
        let mut b = [0u8; 4096];
        while let Ok(Some(n)) = c.read_stderr(&mut b) {
            if n == 0 {
                break;
            }
            if errtext.len() < 4096 {
                errtext.extend_from_slice(&b[..n]);
            }
        }
        while let Ok(Some(n)) = c.read_stdout(&mut b) {
            if n == 0 {
                break;
            }
        }
        match c.try_wait() {
            Ok(Some(0)) => return Ok(()),
            Ok(Some(code)) => return Err(format!("{} endete mit {}: {}", program, code, String::from_utf8_lossy(&errtext).trim())),
            Ok(None) => {}
            Err(e) => return Err(format!("{}: Fehler {}", program, e)),
        }
        if plat::time_us() - start > timeout_s as u64 * 1_000_000 {
            c.terminate();
            return Err(format!("{} braucht zu lange", program));
        }
        plat::sleep_ms(50);
    }
}

// ------------------------------------------------------------------ inventory

static SCANNED: Mutex<i64> = Mutex::new(0);

/// look at the QMP sockets again: VMs found are running (or paused), known ones not found stopped;
/// a socket another client holds (a backup) keeps its VM as it was
pub fn scan(ctx: &Ctx) -> Vec<Vm> {
    let Some(host) = server_host(ctx) else { return Vec::new() };
    let t = now();
    let mut found: BTreeMap<String, Vm> = BTreeMap::new();
    let known: Vec<Vm> = all(&ctx.db, C_VMS);
    for f in folders(ctx) {
        for e in plat::fs::read_dir(&f).unwrap_or_default() {
            let path = plat::fs::join(&f, &e.name);
            let mut q = match bc_vm::reach(&path, 3000) {
                bc_vm::Reach::Up(q) => q,
                bc_vm::Reach::Busy => {
                    if let Some(v) = known.iter().find(|v| v.qmp == path) {
                        found.insert(v.id.clone(), v.clone());
                    }
                    continue;
                }
                bc_vm::Reach::Down => continue,
            };
            let Ok(i) = bc_vm::info(&mut q) else { continue };
            let id = format!("{}:{}", host.id, i.name);
            found.insert(id.clone(), Vm {
                id,
                name: i.name,
                uuid: i.uuid,
                host_id: host.id.clone(),
                host_name: host.name.clone(),
                state: i.status,
                qmp: path,
                memory_mb: i.memory_mb,
                cpus: i.cpus,
                disks: i.disks.into_iter().map(|d| VmDisk { node: d.node, file: d.file, format: d.format, size: d.size, bus: d.bus }).collect(),
                version: i.version,
                seen: t,
            });
        }
    }
    for mut v in known {
        if !found.contains_key(&v.id) {
            v.state = String::from("stopped");
            found.insert(v.id.clone(), v);
        }
    }
    for v in found.values() {
        let _ = ctx.db.put(C_VMS, &v.id, v.to_json());
    }
    *SCANNED.lock() = t;
    let mut v: Vec<Vm> = found.into_values().collect();
    v.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    v
}

fn fresh(ctx: &Ctx) -> Vec<Vm> {
    if now() - *SCANNED.lock() >= 10 {
        return scan(ctx);
    }
    let mut v: Vec<Vm> = all(&ctx.db, C_VMS);
    v.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    v
}

pub fn list_vms(ctx: &Ctx) -> Response {
    list(&fresh(ctx))
}

pub fn status(ctx: &Ctx) -> Response {
    let vms = fresh(ctx);
    let Some(host) = server_host(ctx) else { return list::<Hypervisor>(&[]) };
    let f = folders(ctx);
    let linux = cfg!(target_os = "linux");
    let version = vms.iter().find(|v| !v.version.is_empty()).map_or(String::new(), |v| v.version.clone());
    let message = if !linux {
        String::from("QEMU/KVM gibt es nur auf einem Linux-Server")
    } else if f.is_empty() {
        String::from("Keine QMP-Ordner eingestellt (hypervisor.qmp)")
    } else {
        String::new()
    };
    let h = Hypervisor {
        host_id: host.id.clone(),
        host_name: host.name.clone(),
        kind: String::from("qemu"),
        version,
        kvm: kvm(),
        folders: f.clone(),
        vms: vms.len() as u32,
        running: vms.iter().filter(|v| v.state == "running").count() as u32,
        ok: linux && !f.is_empty(),
        message,
    };
    list(&[h])
}

// ------------------------------------------------------------------ VM jobs

/// the counts of a VM (or volume) backup run
pub(crate) struct Ingest {
    pub done: u64,
    pub total: u64,
    pub files: u64,
}

fn blocks(size: u64) -> u64 {
    size.div_ceil(BLOCK)
}

pub(crate) fn file_node(name: &str, size: u64, chunks: Vec<cas::ChunkId>) -> Node {
    let t = plat::unix_time_ns();
    Node { name: String::from(name), kind: NodeKind::File, size, chunks, mtime_ns: t, atime_ns: t, ctime_ns: t, mode: 0o600, ..Node::default() }
}

/// a raw image into blocks
fn ingest_raw(o: &OpCtl, w: &mut cas::Writer, name: &str, path: &str, size: u64, st: &mut Ingest) -> Result<Node, String> {
    let f = plat::File::open(&plat::fs::long_path(path), plat::FO_READ).ok_or_else(|| format!("{}: nicht lesbar", path))?;
    ingest_file(o, w, name, &f, path, size, st)
}

/// an open image or device into blocks (`path` names it in errors)
pub(crate) fn ingest_file(o: &OpCtl, w: &mut cas::Writer, name: &str, f: &plat::File, path: &str, size: u64, st: &mut Ingest) -> Result<Node, String> {
    let mut buf = alloc::vec![0u8; BLOCK as usize];
    let mut chunks = Vec::with_capacity(blocks(size) as usize);
    let mut off = 0u64;
    while off < size {
        if o.cancelled() {
            return Err(String::from("Abgebrochen"));
        }
        let n = (size - off).min(BLOCK) as usize;
        let got = f.read_at(off, &mut buf[..n]).ok_or_else(|| format!("{}: Lesefehler", path))?;
        // a shorter file reads as zeros at its end (sparse images)
        buf[got..n].fill(0);
        chunks.push(w.put(KIND_DATA, &buf[..n]).map_err(engine::err_text)?);
        off += n as u64;
        st.done += n as u64;
        o.progress(st.done, st.total.max(st.done));
    }
    st.files += 1;
    Ok(file_node(name, size, chunks))
}

/// an incremental dump (qcow2 of the dirty clusters) over the previous point's blocks
fn ingest_dirty(o: &OpCtl, repo: &Repo, w: &mut cas::Writer, name: &str, path: &str, prev: &Node, size: u64, st: &mut Ingest, changed: &mut u64) -> Result<Node, String> {
    let img = bc_vm::qcow2::Qcow2::open(path)?;
    if img.size != size {
        return Err(format!("{}: die Größe des Abbilds passt nicht", name));
    }
    let cs = img.cluster();
    let mut by_block: BTreeMap<u64, Vec<(u64, Option<u64>)>> = BTreeMap::new();
    for (g, h) in img.present()? {
        by_block.entry(g / BLOCK).or_default().push((g, h));
    }
    let mut chunks = prev.chunks.clone();
    let mut dirty = 0u64;
    for (b, clusters) in &by_block {
        if o.cancelled() {
            return Err(String::from("Abgebrochen"));
        }
        let start = b * BLOCK;
        let len = (size - start).min(BLOCK) as usize;
        let mut data = repo.read_chunk(&chunks[*b as usize]).map_err(engine::err_text)?;
        data.resize(len, 0);
        for (g, h) in clusters {
            let at = (g - start) as usize;
            let n = (cs as usize).min(len - at);
            match h {
                Some(host) => {
                    let c = img.read(*host)?;
                    data[at..at + n].copy_from_slice(&c[..n]);
                }
                None => data[at..at + n].fill(0),
            }
        }
        chunks[*b as usize] = w.put(KIND_DATA, &data).map_err(engine::err_text)?;
        dirty += len as u64;
        st.done += len as u64;
        o.progress(st.done, st.total.max(st.done));
    }
    st.done += size - dirty;
    *changed += by_block.len() as u64;
    st.files += 1;
    Ok(file_node(name, size, chunks))
}

pub(crate) fn json_node(w: &mut cas::Writer, name: &str, v: &Value) -> Result<Node, String> {
    let b = j::text(v);
    let id = w.put(KIND_DATA, b.as_bytes()).map_err(engine::err_text)?;
    Ok(file_node(name, b.len() as u64, alloc::vec![id]))
}

#[allow(clippy::too_many_arguments)]
fn backup_vm(o: &OpCtl, ctx: &Ctx, repo: &Repo, w: &mut cas::Writer, job: &Job, vm: &Vm, prev: Option<&Node>, full: bool, spool: &str, s: &mut Session, st: &mut Ingest) -> Result<Node, String> {
    let prev_tree: Option<Tree> = prev.and_then(|n| n.tree).and_then(|t| repo.read_tree(&t).ok());
    let prev_cfg = prev_tree.as_ref().and_then(|t| t.find("vm.json")).and_then(|n| read_small(repo, n));
    let was_cold = prev_cfg.as_ref().and_then(|c| c.get("cold")).and_then(|v| v.as_bool()).unwrap_or(false);
    let tag: String = new_id().chars().take(8).collect();
    let bitmap = format!("backupcraft-{}", job.id.chars().take(8).collect::<String>());
    let mut nodes: Vec<Node> = Vec::new();
    let disks: Vec<VmDisk>;
    let (mut memory, mut cpus, mut cold) = (vm.memory_mb, vm.cpus, false);
    let qmp = if vm.qmp.is_empty() {
        None
    } else {
        match bc_vm::reach(&vm.qmp, 5000) {
            bc_vm::Reach::Up(q) => Some(q),
            bc_vm::Reach::Busy => return Err(String::from("Der QMP-Socket der VM ist belegt (ein anderes Programm ist verbunden)")),
            bc_vm::Reach::Down => None,
        }
    };
    match qmp {
        Some(mut q) => {
            let info = bc_vm::info(&mut q)?;
            memory = info.memory_mb;
            cpus = info.cpus;
            let names: Vec<String> = (0..info.disks.len()).map(|k| format!("disk{}.raw", k)).collect();
            let prev_nodes: Vec<Option<Node>> = names.iter().zip(&info.disks).map(|(n, d)| {
                prev_tree.as_ref().and_then(|t| t.find(n)).filter(|x| x.size == d.size && x.chunks.len() as u64 == blocks(d.size)).cloned()
            }).collect();
            let need_full = full || was_cold || prev_nodes.iter().any(|p| p.is_none());
            engine::session_log(ctx, s, LogLevel::Info, &format!("{}: läuft – {}", vm.name, if need_full { "Vollsicherung durch QEMU" } else { "inkrementell (geänderte Blöcke)" }));
            let dumps = bc_vm::backup_disks(&mut q, &info.disks, &bitmap, need_full, spool, &tag)?;
            // QEMU serves one QMP client: free the socket while the dumps are read
            drop(q);
            let mut changed = 0u64;
            let mut res = Ok(());
            for (k, d) in dumps.iter().enumerate() {
                if res.is_ok() {
                    let r = match (&prev_nodes[k], d.incremental) {
                        (Some(p), true) => ingest_dirty(o, repo, w, &names[k], &d.path, p, d.size, st, &mut changed),
                        _ => ingest_raw(o, w, &names[k], &d.path, d.size, st),
                    };
                    match r {
                        Ok(n) => nodes.push(n),
                        Err(e) => res = Err(e),
                    }
                }
                plat::delete(&d.path);
            }
            res?;
            if !need_full {
                let total: u64 = info.disks.iter().map(|d| blocks(d.size)).sum();
                engine::session_log(ctx, s, LogLevel::Info, &format!("{}: {} von {} Blöcken geändert", vm.name, changed, total));
            }
            disks = info.disks.into_iter().map(|d| VmDisk { node: d.node, file: d.file, format: d.format, size: d.size, bus: d.bus }).collect();
        }
        None => {
            if vm.disks.is_empty() {
                return Err(String::from("Die VM läuft nicht, und ihre Platten sind nicht bekannt"));
            }
            cold = true;
            engine::session_log(ctx, s, LogLevel::Info, &format!("{}: läuft nicht – Sicherung aus den Plattendateien", vm.name));
            for (k, d) in vm.disks.iter().enumerate() {
                let name = format!("disk{}.raw", k);
                let (src, temp) = if d.format == "raw" {
                    (d.file.clone(), false)
                } else {
                    let out = plat::fs::join(spool, &format!("{}-{}.raw", tag, k));
                    run_tool(&qemu_img(ctx), &["convert", "-q", "-O", "raw", &d.file, &out], 24 * 3600)?;
                    (out, true)
                };
                let r = ingest_raw(o, w, &name, &src, d.size, st);
                if temp {
                    plat::delete(&src);
                }
                nodes.push(r?);
            }
            disks = vm.disks.clone();
        }
    }
    let cfg = j::obj([
        ("name", Value::string(vm.name.as_str())),
        ("uuid", Value::string(vm.uuid.as_str())),
        ("memory_mb", Value::u64(memory)),
        ("cpus", Value::u64(cpus as u64)),
        ("block", Value::u64(BLOCK)),
        ("cold", Value::Bool(cold)),
        ("version", Value::string(vm.version.as_str())),
        ("disks", Value::Array(disks.iter().enumerate().map(|(k, d)| {
            let mut v = d.to_json();
            v.insert("name", Value::string(format!("disk{}.raw", k)));
            v
        }).collect())),
    ]);
    let mut all_nodes = alloc::vec![json_node(w, "vm.json", &cfg)?];
    let size: u64 = nodes.iter().map(|n| n.size).sum();
    let count = nodes.len() as u64 + 1;
    all_nodes.extend(nodes);
    let tid = w.put_tree(&Tree { nodes: all_nodes }).map_err(engine::err_text)?;
    let t = plat::unix_time_ns();
    Ok(Node { name: vm.name.clone(), kind: NodeKind::Dir, tree: Some(tid), size, count, mtime_ns: t, atime_ns: t, ctime_ns: t, mode: 0o700, ..Node::default() })
}

/// a small file of a point (vm.json)
fn read_small(repo: &Repo, n: &Node) -> Option<Value> {
    let mut b = Vec::new();
    for c in &n.chunks {
        b.extend_from_slice(&repo.read_chunk(c).ok()?);
        if b.len() > (1 << 20) {
            return None;
        }
    }
    j::parse(&b)
}

fn run_backup(o: &OpCtl, ctx: &Ctx, job: &Job, r: &Repository, full: bool, s: &mut Session) -> Result<String, String> {
    let repo = engine::open_repo(ctx, r)?;
    engine::ensure_job_key(ctx, job, &repo, s)?;
    let prev = engine::previous(ctx, job, &r.id);
    let prev_snap = prev.as_ref().and_then(|p| repo.load_snapshot(&p.id).ok());
    let spool = plat::fs::join(&work(ctx), "spool");
    bc_fs::ensure_dir(&spool)?;
    let vms: Vec<Option<Vm>> = job.sources.iter().map(|id| rec::<Vm>(ctx.get(C_VMS, id))).collect();
    let total: u64 = vms.iter().flatten().map(|v| v.disks.iter().map(|d| d.size).sum::<u64>()).sum();
    s.bytes_total = total;
    let mut st = Ingest { done: 0, total, files: 0 };
    let (codec, level) = engine::codec_of(job.compression);
    let mut w = repo.writer().codec(codec, level);
    let mut roots = Vec::new();
    for (k, v) in vms.iter().enumerate() {
        let Some(vm) = v else {
            s.errors += 1;
            engine::session_log(ctx, s, LogLevel::Error, &format!("VM {} ist nicht (mehr) bekannt", job.sources[k]));
            continue;
        };
        let prev_root = prev_snap.as_ref().and_then(|p| p.roots.iter().find(|n| n.name == vm.name)).cloned();
        match backup_vm(o, ctx, &repo, &mut w, job, vm, prev_root.as_ref(), full, &spool, s, &mut st) {
            Ok(n) => roots.push(n),
            Err(e) if e == "Abgebrochen" => return Err(e),
            Err(e) => {
                s.errors += 1;
                engine::session_log(ctx, s, LogLevel::Error, &format!("{}: {}", vm.name, e));
            }
        }
    }
    let ws = w.finish().map_err(engine::err_text)?;
    if roots.is_empty() {
        return Err(String::from("Keine VM wurde gesichert"));
    }
    s.files_total = st.files;
    s.files_done = st.files;
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
        full: full || prev.is_none(),
        stats: cas::SnapStats { files: st.files, dirs: 0, bytes, new_bytes: ws.new_stored, new_chunks: ws.new_chunks, new_raw: ws.new_raw, errors: s.errors as u64 },
        immutable_until: if imm > 0 { t + imm as i64 * DAY } else { 0 },
        meta: j::obj([("session", Value::string(s.id.as_str())), ("created_ns", Value::i64(plat::unix_time_ns())), ("kind", Value::string("vm"))]),
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
    let msg = format!("{} VMs, {} Platten ({}), {} neu gespeichert", snap.roots.len(), st.files, size(bytes), size(ws.new_stored));
    if s.errors > 0 { Ok(format!("{} · {} Fehler", msg, s.errors)) } else { Ok(msg) }
}

pub fn start_job(ctx: &Arc<Ctx>, job: &Job, full: bool, actor: &str) -> Result<String, String> {
    let r: Repository = rec(ctx.get(C_REPOS, &job.repository_id)).ok_or("Das Repository des Jobs fehlt")?;
    let mut session = engine::session_for(ctx, SessionKind::Backup, job, "");
    engine::set_status(ctx, &job.id, SessionStatus::Running, None);
    let job = job.clone();
    let sid = session.id.clone();
    let op = Ops::start(ctx, "job.run", &format!("{}: Sicherung", job.name), actor, &job.id.clone(), &sid, Box::new(move |o| {
        let ctx = o.ctx().clone();
        engine::session_log(&ctx, &session, LogLevel::Info, &format!("Sitzung gestartet: {}", job.name));
        let res = run_backup(o, &ctx, &job, &r, full, &mut session);
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

/// the VM ids of a job that are not known
pub fn unknown(ctx: &Ctx, ids: &[String]) -> Option<String> {
    ids.iter().find(|id| !ctx.db.contains(C_VMS, id)).cloned()
}

// ------------------------------------------------------------------ a VM of a point

/// what a point holds of one VM
struct PointVm {
    repo: Arc<Repo>,
    name: String,
    memory_mb: u64,
    cpus: u32,
    /// (export name, bus, file node)
    disks: Vec<(String, String, Node)>,
}

fn point_vm(ctx: &Ctx, p: &RestorePoint, vm: &str) -> Result<PointVm, String> {
    let (_, repo) = engine::repo_for(ctx, &p.repository_id)?;
    let snap = repo.load_snapshot(&p.id).map_err(engine::err_text)?;
    let root = snap.roots.iter().find(|r| r.name == vm).ok_or_else(|| format!("Der Restore-Point enthält keine VM „{}“", vm))?;
    let tree = root.tree.ok_or("keine VM").and_then(|t| repo.read_tree(&t).map_err(|_| "unlesbar"))?;
    let cfg = tree.find("vm.json").and_then(|n| read_small(&repo, n)).ok_or_else(|| format!("„{}“ ist keine gesicherte VM", vm))?;
    if j::u64_of(&cfg, "block") != BLOCK {
        return Err(String::from("Unbekannte Blockgröße der VM-Sicherung"));
    }
    let mut disks = Vec::new();
    for (k, d) in cfg.get("disks").and_then(|v| v.as_array()).unwrap_or(&[]).iter().enumerate() {
        let n = tree.find(j::str_of(d, "name")).cloned().ok_or("eine Platte fehlt im Restore-Point")?;
        disks.push((format!("disk{}", k), String::from(j::str_of(d, "bus")), n));
    }
    if disks.is_empty() {
        return Err(String::from("Die VM hat keine Platten"));
    }
    Ok(PointVm { repo, name: String::from(vm), memory_mb: j::u64_of(&cfg, "memory_mb").max(16), cpus: (j::u64_of(&cfg, "cpus") as u32).max(1), disks })
}

/// the NBD export of a disk of a point: block i is chunk i (the last few blocks are cached)
fn export(repo: Arc<Repo>, name: &str, node: Node) -> Export {
    let cache: Arc<Mutex<Vec<(u64, Arc<Vec<u8>>)>>> = Arc::new(Mutex::new(Vec::new()));
    let size = node.size;
    let read = move |off: u64, len: usize| -> Result<Vec<u8>, String> {
        let mut out = Vec::with_capacity(len);
        let end = off + len as u64;
        let mut pos = off;
        while pos < end {
            let b = pos / BLOCK;
            let hit = cache.lock().iter().find(|(k, _)| *k == b).map(|(_, d)| d.clone());
            let data = match hit {
                Some(d) => d,
                None => {
                    let id = node.chunks.get(b as usize).ok_or("Block fehlt")?;
                    let d = Arc::new(repo.read_chunk(id).map_err(engine::err_text)?);
                    let mut c = cache.lock();
                    if c.len() >= 16 {
                        c.remove(0);
                    }
                    c.push((b, d.clone()));
                    d
                }
            };
            let at = (pos - b * BLOCK) as usize;
            let n = (data.len().saturating_sub(at) as u64).min(end - pos) as usize;
            if n == 0 {
                return Err(String::from("Block zu kurz"));
            }
            out.extend_from_slice(&data[at..at + n]);
            pos += n as u64;
        }
        Ok(out)
    };
    Export { name: String::from(name), size, read: Arc::new(read) }
}

// ------------------------------------------------------------------ running VMs

/// a VM this server started
struct Live {
    child: Mutex<Option<plat::process::Child>>,
    nbd: Mutex<Option<NbdServer>>,
    dir: String,
    qmp: String,
    console: String,
    /// stopped by us (not a crash)
    stopping: core::sync::atomic::AtomicBool,
}

static LIVE: Mutex<BTreeMap<String, Arc<Live>>> = Mutex::new(BTreeMap::new());

fn session(ctx: &Ctx, id: &str) -> Option<InstantSession> {
    rec(ctx.get(C_INSTANT, id))
}

fn set_session(ctx: &Ctx, id: &str, f: impl FnOnce(&mut InstantSession)) {
    if let Some(mut s) = session(ctx, id) {
        f(&mut s);
        let _ = ctx.db.put(C_INSTANT, id, s.to_json());
    }
}

fn device(bus: &str, k: usize) -> String {
    match bus {
        "virtio" => format!("virtio-blk-pci,drive=disk{},bootindex={}", k, k),
        "scsi" => format!("scsi-hd,drive=disk{},bus=scsi0.0,bootindex={}", k, k),
        _ => format!("ide-hd,drive=disk{},bootindex={}", k, k),
    }
}

/// a free TCP port on loopback
fn free_port() -> Option<u16> {
    let l = plat::net::TcpListener::bind(&plat::net::SocketAddr::parse("127.0.0.1:0")?).ok()?;
    l.local_addr().ok().map(|a| a.port)
}

/// boot a VM of a point: NBD export, overlays, QEMU -> the running VM
fn launch(ctx: &Arc<Ctx>, sid: &str, pv: &PointVm, memory_mb: u64, network: &str, forward: Option<(u16, u16)>) -> Result<Arc<Live>, String> {
    if !cfg!(target_os = "linux") {
        return Err(String::from("Der Sofortstart braucht einen Linux-Server mit QEMU/KVM"));
    }
    let dir = plat::fs::join(&plat::fs::join(&work(ctx), "instant"), sid);
    bc_fs::ensure_dir(&dir)?;
    let sock = plat::fs::join(&dir, "nbd.sock");
    let exports: Vec<Export> = pv.disks.iter().map(|(name, _, n)| export(pv.repo.clone(), name, n.clone())).collect();
    let server = nbd::serve(&sock, exports)?;
    let mut args: Vec<String> = Vec::new();
    if kvm() {
        args.push(String::from("-enable-kvm"));
    }
    for a in ["-name", &format!("{}-instant", pv.name), "-m", &format!("{}", memory_mb), "-smp", &format!("{}", pv.cpus), "-display", "none"] {
        args.push(String::from(a));
    }
    let qmp = plat::fs::join(&dir, "qmp.sock");
    let console = plat::fs::join(&dir, "console.log");
    args.push(String::from("-qmp"));
    args.push(format!("unix:{},server=on,wait=off", qmp));
    args.push(String::from("-serial"));
    args.push(format!("file:{}", console));
    args.push(String::from("-nic"));
    args.push(match (network, forward) {
        ("isolated", Some((host, guest))) => format!("user,restrict=on,hostfwd=tcp:127.0.0.1:{}-:{}", host, guest),
        ("isolated", None) => String::from("user,restrict=on"),
        _ => String::from("none"),
    });
    if pv.disks.iter().any(|(_, b, _)| b == "scsi") {
        args.push(String::from("-device"));
        args.push(String::from("virtio-scsi-pci,id=scsi0"));
    }
    for (k, (name, bus, _)) in pv.disks.iter().enumerate() {
        let overlay = plat::fs::join(&dir, &format!("{}.qcow2", name));
        let backing = format!("nbd+unix:///{}?socket={}", name, sock);
        run_tool(&qemu_img(ctx), &["create", "-q", "-f", "qcow2", "-F", "raw", "-b", &backing, &overlay], 60)?;
        args.push(String::from("-blockdev"));
        args.push(format!("driver=qcow2,node-name=disk{},file.driver=file,file.filename={}", k, overlay));
        args.push(String::from("-device"));
        args.push(device(bus, k));
    }
    let refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let program = qemu(ctx);
    let mut child = plat::process::Command { program: &program, args: &refs, cwd: Some(&dir), env: &[] }.spawn().map_err(|e| format!("QEMU startet nicht (Fehler {})", e))?;
    // up when its QMP socket answers
    let start = plat::time_us();
    loop {
        if let Ok(Some(code)) = child.try_wait() {
            let mut b = [0u8; 4096];
            let n = child.read_stderr(&mut b).ok().flatten().unwrap_or(0);
            return Err(format!("QEMU endete gleich wieder ({}): {}", code, String::from_utf8_lossy(&b[..n]).trim()));
        }
        if plat::fs::lstat(&qmp).is_some() && Qmp::connect(&qmp, 1000).is_ok() {
            break;
        }
        if plat::time_us() - start > 20_000_000 {
            child.terminate();
            return Err(String::from("QEMU meldet sich nicht"));
        }
        plat::sleep_ms(100);
    }
    let live = Arc::new(Live { child: Mutex::new(Some(child)), nbd: Mutex::new(Some(server)), dir, qmp, console, stopping: core::sync::atomic::AtomicBool::new(false) });
    LIVE.lock().insert(String::from(sid), live.clone());
    // keep the pipes empty, note the end of the VM
    let (c, id, l2) = (ctx.clone(), String::from(sid), live.clone());
    let _ = plat::thread_spawn(64 * 1024, Box::new(move || watch(&c, &id, &l2)));
    Ok(live)
}

fn watch(ctx: &Ctx, sid: &str, live: &Live) {
    let log = plat::fs::join(&live.dir, "qemu.log");
    loop {
        let mut out = Vec::new();
        let ended = {
            let mut g = live.child.lock();
            let Some(c) = g.as_mut() else { return };
            let mut b = [0u8; 4096];
            while let Ok(Some(n)) = c.read_stderr(&mut b) {
                if n == 0 {
                    break;
                }
                out.extend_from_slice(&b[..n]);
            }
            while let Ok(Some(n)) = c.read_stdout(&mut b) {
                if n == 0 {
                    break;
                }
                out.extend_from_slice(&b[..n]);
            }
            c.try_wait().ok().flatten()
        };
        if !out.is_empty() {
            let mut old = plat::read_file(&log).unwrap_or_default();
            if old.len() < (1 << 20) {
                old.extend_from_slice(&out);
                let _ = plat::fs::write_file_private(&log, &old);
            }
        }
        if let Some(code) = ended {
            *live.nbd.lock() = None;
            LIVE.lock().remove(sid);
            if !live.stopping.load(core::sync::atomic::Ordering::SeqCst) {
                set_session(ctx, sid, |s| {
                    if !matches!(s.state.as_str(), "stopped") {
                        s.state = String::from(if code == 0 { "stopped" } else { "failed" });
                        s.stopped = now();
                        s.message = if code == 0 { String::from("Die VM wurde im Gast heruntergefahren") } else { format!("QEMU endete mit {}", code) };
                    }
                });
            }
            return;
        }
        plat::sleep_ms(500);
    }
}

/// end a VM: quit through QMP, else kill; the overlays go unless keep
fn shutdown(sid: &str, keep: bool) {
    let live = LIVE.lock().remove(sid);
    let Some(live) = live else { return };
    live.stopping.store(true, core::sync::atomic::Ordering::SeqCst);
    if let Ok(mut q) = Qmp::connect(&live.qmp, 2000) {
        let _ = q.cmd("quit", Value::Null);
    }
    let start = plat::time_us();
    loop {
        let done = live.child.lock().as_mut().is_none_or(|c| c.try_wait().ok().flatten().is_some());
        if done || plat::time_us() - start > 5_000_000 {
            break;
        }
        plat::sleep_ms(100);
    }
    if let Some(mut c) = live.child.lock().take() {
        c.terminate();
    }
    *live.nbd.lock() = None;
    if !keep {
        let _ = bc_fs::remove_tree(&live.dir);
    }
}

pub fn instant_list(ctx: &Ctx) -> Response {
    let mut v: Vec<InstantSession> = all(&ctx.db, C_INSTANT);
    v.sort_by_key(|s| core::cmp::Reverse(s.started));
    v.truncate(100);
    list(&v)
}

fn point_of(ctx: &Ctx, id: &str) -> Option<RestorePoint> {
    rec(ctx.get(C_POINTS, id))
}

pub fn instant_start(ctx: &Arc<Ctx>, c: &Call) -> Response {
    let Some(p) = point_of(ctx, &c.param("id")) else { return not_found("Restore-Point nicht gefunden") };
    let Some(r) = InstantRequest::from_json(&c.body) else { return util::bad("Ungültige Anfrage") };
    if !cfg!(target_os = "linux") {
        return err(409, code::CONFLICT, "Der Sofortstart braucht einen Linux-Server mit QEMU/KVM");
    }
    let pv = match point_vm(ctx, &p, &r.vm) {
        Ok(x) => x,
        Err(e) => return field(code::BAD_REQUEST, &e, "vm"),
    };
    if !matches!(r.network.as_str(), "" | "none" | "isolated") {
        return field(code::BAD_REQUEST, "Netzwerk: none oder isolated", "network");
    }
    let memory = if r.memory_mb == 0 { pv.memory_mb } else { r.memory_mb.clamp(16, 1 << 20) };
    let sid = new_id();
    let s = InstantSession {
        id: sid.clone(),
        point_id: p.id.clone(),
        point_time: p.created,
        job_id: p.job_id.clone(),
        job_name: p.job_name.clone(),
        vm: pv.name.clone(),
        purpose: String::from("instant"),
        state: String::from("starting"),
        memory_mb: memory,
        cpus: pv.cpus,
        network: String::from(if r.network.is_empty() { "none" } else { r.network.as_str() }),
        started: now(),
        owner: c.actor(),
        ..InstantSession::default()
    };
    let _ = ctx.db.put(C_INSTANT, &sid, s.to_json());
    let net = s.network.clone();
    let op = Ops::start(ctx, "instant.start", &format!("{}: {} aus dem Restore-Point starten", p.job_name, pv.name), &c.actor(), &sid.clone(), "", Box::new(move |o| {
        let ctx = o.ctx().clone();
        match launch(&ctx, &sid, &pv, memory, &net, None) {
            Ok(_) => {
                set_session(&ctx, &sid, |s| {
                    s.state = String::from("running");
                    s.message = String::from(if kvm() { "läuft (KVM)" } else { "läuft (ohne Hardware-Virtualisierung)" });
                });
                Ok(format!("{} läuft aus dem Restore-Point vom {}", pv.name, crate::notify::when(p.created)))
            }
            Err(e) => {
                set_session(&ctx, &sid, |s| {
                    s.state = String::from("failed");
                    s.stopped = now();
                    s.message = e.clone();
                });
                shutdown(&sid, false);
                Err(e)
            }
        }
    }));
    c.audit("instant.start", &format!("{} ({})", r.vm, c.param("id")), "");
    util::accepted(&op)
}

pub fn instant_stop(ctx: &Ctx, c: &Call) -> Response {
    let id = c.param("id");
    let Some(s) = session(ctx, &id) else { return not_found("Sitzung nicht gefunden") };
    let migrated = s.state == "migrated";
    shutdown(&id, migrated);
    set_session(ctx, &id, |x| {
        x.state = String::from("stopped");
        x.stopped = now();
        x.message = String::from(if migrated { "Beendet; die zusammengeführten Platten bleiben" } else { "Beendet; die Änderungen wurden verworfen" });
    });
    c.audit("instant.stop", &s.vm, "");
    ok(200, &session(ctx, &id).unwrap_or(s))
}

pub fn instant_console(ctx: &Ctx, c: &Call) -> Response {
    let id = c.param("id");
    let Some(_s) = session(ctx, &id) else { return not_found("Sitzung nicht gefunden") };
    let path = plat::fs::join(&plat::fs::join(&plat::fs::join(&work(ctx), "instant"), &id), "console.log");
    let b = plat::read_file(&path).unwrap_or_default();
    let tail = &b[b.len().saturating_sub(16 * 1024)..];
    util::json(200, &j::obj([("text", Value::string(String::from_utf8_lossy(tail).into_owned()))]))
}

pub fn instant_migrate(ctx: &Arc<Ctx>, c: &Call) -> Response {
    let id = c.param("id");
    let Some(s) = session(ctx, &id) else { return not_found("Sitzung nicht gefunden") };
    let Some(r) = MigrateRequest::from_json(&c.body) else { return util::bad("Ungültige Anfrage") };
    if s.state != "running" || !LIVE.lock().contains_key(&id) {
        return err(409, code::CONFLICT, "Die VM läuft nicht aus einem Restore-Point");
    }
    let target = String::from(r.target.trim());
    let Some(host) = server_host(ctx) else { return util::internal("") };
    if !bc_fs::absolute(&target, cfg!(windows)) || !bc_fs::inside_os(&crate::restore::allowed_roots(ctx, &host), &target, cfg!(windows)) {
        return field(code::FORBIDDEN, "Der Zielordner muss in einem Wiederherstellungsordner des Servers liegen", "target");
    }
    set_session(ctx, &id, |x| x.state = String::from("migrating"));
    let disks = point_of(ctx, &s.point_id).and_then(|p| point_vm(ctx, &p, &s.vm).ok()).map_or(0, |pv| pv.disks.len());
    let vm = s.vm.clone();
    let op = Ops::start(ctx, "instant.migrate", &format!("{}: zusammenführen", s.vm), &c.actor(), &id.clone(), "", Box::new(move |o| {
        let ctx = o.ctx().clone();
        let res = migrate(o, &id, &vm, disks, &target);
        match &res {
            Ok(_) => set_session(&ctx, &id, |x| {
                x.state = String::from("migrated");
                x.target = target.clone();
                x.message = format!("Läuft von {}", target);
            }),
            Err(e) => set_session(&ctx, &id, |x| {
                x.state = String::from("running");
                x.message = format!("Zusammenführen fehlgeschlagen: {}", e);
            }),
        }
        res
    }));
    c.audit("instant.migrate", &s.vm, &r.target);
    util::accepted(&op)
}

fn migrate(o: &OpCtl, sid: &str, vm: &str, disks: usize, target: &str) -> Result<String, String> {
    let live = LIVE.lock().get(sid).cloned().ok_or("Die VM läuft nicht mehr")?;
    bc_fs::ensure_dir(target)?;
    let mut q = Qmp::connect(&live.qmp, 3000)?;
    let mut files = Vec::new();
    for k in 0..disks {
        let file = plat::fs::join(target, &format!("{}-disk{}.qcow2", vm, k));
        if plat::fs::lstat(&file).is_some() {
            return Err(format!("{} gibt es schon", file));
        }
        q.cmd("drive-mirror", j::obj([("device", Value::string(format!("disk{}", k))), ("job-id", Value::string(format!("mig{}", k))),
                                      ("target", Value::string(file.as_str())), ("format", Value::string("qcow2")), ("sync", Value::string("full")),
                                      ("mode", Value::string("absolute-paths"))]))?;
        files.push(file);
    }
    for k in 0..disks {
        let jid = format!("mig{}", k);
        let ev = q.event(&|v: &Value| matches!(j::str_of(v, "event"), "BLOCK_JOB_READY" | "BLOCK_JOB_COMPLETED" | "BLOCK_JOB_CANCELLED")
                         && v.get("data").is_some_and(|d| j::str_of(d, "device") == jid), u32::MAX)?;
        if j::str_of(&ev, "event") != "BLOCK_JOB_READY" {
            return Err(format!("Spiegeln von Platte {} abgebrochen: {}", k, ev.get("data").map_or("", |d| j::str_of(d, "error"))));
        }
        o.progress(k as u64 + 1, disks as u64 * 2);
    }
    for k in 0..disks {
        q.cmd("block-job-complete", j::obj([("device", Value::string(format!("mig{}", k)))]))?;
    }
    for k in 0..disks {
        let jid = format!("mig{}", k);
        let ev = q.event(&|v: &Value| matches!(j::str_of(v, "event"), "BLOCK_JOB_COMPLETED" | "BLOCK_JOB_CANCELLED")
                         && v.get("data").is_some_and(|d| j::str_of(d, "device") == jid), 600_000)?;
        if !j::str_of(ev.get("data").unwrap_or(&Value::Null), "error").is_empty() {
            return Err(format!("Umschalten von Platte {} fehlgeschlagen", k));
        }
        o.progress(disks as u64 + k as u64 + 1, disks as u64 * 2);
    }
    // the VM runs from the new files: the export is no longer needed
    *live.nbd.lock() = None;
    Ok(format!("{} läuft jetzt von {}", vm, files.join(", ")))
}

// ------------------------------------------------------------------ boot checks

pub fn bootcheck_list(ctx: &Ctx) -> Response {
    let mut v: Vec<BootCheckResult> = all(&ctx.db, C_BOOTCHECK);
    v.sort_by_key(|s| core::cmp::Reverse(s.started));
    v.truncate(200);
    list(&v)
}

pub fn bootcheck_start(ctx: &Arc<Ctx>, c: &Call) -> Response {
    let Some(p) = point_of(ctx, &c.param("id")) else { return not_found("Restore-Point nicht gefunden") };
    let Some(r) = BootCheckRequest::from_json(&c.body) else { return util::bad("Ungültige Anfrage") };
    if !cfg!(target_os = "linux") {
        return err(409, code::CONFLICT, "Die Startprüfung braucht einen Linux-Server mit QEMU/KVM");
    }
    let pv = match point_vm(ctx, &p, &r.vm) {
        Ok(x) => x,
        Err(e) => return field(code::BAD_REQUEST, &e, "vm"),
    };
    if r.tcp_port > 65535 {
        return field(code::BAD_REQUEST, "Ungültiger Port", "tcp_port");
    }
    let job = rec::<Job>(ctx.get(C_JOBS, &p.job_id)).unwrap_or(Job { id: p.job_id.clone(), name: p.job_name.clone(), ..Job::default() });
    let mut session = engine::session_for(ctx, SessionKind::Verify, &job, &p.id);
    let sid = session.id.clone();
    let actor = c.actor();
    let op = Ops::start(ctx, "bootcheck", &format!("{}: Startprüfung {}", p.job_name, pv.name), &actor.clone(), &p.id.clone(), &sid, Box::new(move |o| {
        let ctx = o.ctx().clone();
        let res = boot_check(o, &ctx, &p, pv, &r, &actor, &mut session);
        engine::finish_session(&ctx, &mut session, &res, o.cancelled());
        res
    }));
    c.audit("bootcheck", &c.param("id"), "");
    util::accepted(&op)
}

fn boot_check(o: &OpCtl, ctx: &Arc<Ctx>, p: &RestorePoint, pv: PointVm, r: &BootCheckRequest, actor: &str, s: &mut Session) -> Result<String, String> {
    let sid = new_id();
    let boot = r.boot_seconds.clamp(1, 600) as u64;
    let timeout = if r.timeout_s == 0 { 180 } else { r.timeout_s.clamp(5, 3600) } as u64;
    let fwd = if r.tcp_port > 0 { Some((free_port().ok_or("kein freier Port")?, r.tcp_port as u16)) } else { None };
    let is = InstantSession {
        id: sid.clone(),
        point_id: p.id.clone(),
        point_time: p.created,
        job_id: p.job_id.clone(),
        job_name: p.job_name.clone(),
        vm: pv.name.clone(),
        purpose: String::from("bootcheck"),
        state: String::from("starting"),
        memory_mb: pv.memory_mb,
        cpus: pv.cpus,
        network: String::from(if fwd.is_some() { "isolated" } else { "none" }),
        started: now(),
        owner: String::from(actor),
        ..InstantSession::default()
    };
    let _ = ctx.db.put(C_INSTANT, &sid, is.to_json());
    let mut result = BootCheckResult {
        id: new_id(),
        point_id: p.id.clone(),
        point_time: p.created,
        job_id: p.job_id.clone(),
        job_name: p.job_name.clone(),
        vm: pv.name.clone(),
        started: now(),
        ..BootCheckResult::default()
    };
    engine::session_log(ctx, s, LogLevel::Info, &format!("Startprüfung: {} startet ohne Netzwerkzugang", pv.name));
    let live = match launch(ctx, &sid, &pv, pv.memory_mb, if fwd.is_some() { "isolated" } else { "none" }, fwd) {
        Ok(l) => l,
        Err(e) => {
            result.checks.push(CheckResult { name: String::from("Start"), ok: false, detail: e.clone() });
            return finish_check(ctx, &sid, result, s, Err(e));
        }
    };
    set_session(ctx, &sid, |x| x.state = String::from("running"));
    // 1: it runs for `boot` seconds without failing
    let start = plat::time_us();
    let mut q = Qmp::connect(&live.qmp, 3000).ok();
    let mut ok_boot = true;
    let mut why = String::new();
    while plat::time_us() - start < boot * 1_000_000 {
        if o.cancelled() {
            shutdown(&sid, false);
            return finish_check(ctx, &sid, result, s, Err(String::from("Abgebrochen")));
        }
        if !LIVE.lock().contains_key(&sid) {
            ok_boot = false;
            why = String::from("QEMU endete");
            break;
        }
        if let Some(qq) = q.as_mut() {
            match qq.cmd("query-status", Value::Null) {
                Ok(v) if matches!(j::str_of(&v, "status"), "running") => {}
                Ok(v) => {
                    ok_boot = false;
                    why = format!("Zustand {}", j::str_of(&v, "status"));
                    break;
                }
                Err(e) => {
                    ok_boot = false;
                    why = e;
                    break;
                }
            }
            if qq.take_events().iter().any(|e| matches!(j::str_of(e, "event"), "GUEST_PANICKED" | "SHUTDOWN")) {
                ok_boot = false;
                why = String::from("Der Gast stürzte ab oder fuhr herunter");
                break;
            }
        }
        plat::sleep_ms(500);
    }
    result.checks.push(CheckResult { name: String::from("Start"), ok: ok_boot, detail: if ok_boot { format!("läuft seit {} s", boot) } else { why } });
    // 2: the console shows the text
    if !r.console_text.is_empty() {
        let mut seen = false;
        while plat::time_us() - start < timeout * 1_000_000 && !o.cancelled() {
            let b = plat::read_file(&live.console).unwrap_or_default();
            if String::from_utf8_lossy(&b).contains(r.console_text.as_str()) {
                seen = true;
                break;
            }
            plat::sleep_ms(500);
        }
        result.checks.push(CheckResult { name: String::from("Konsole"), ok: seen, detail: if seen { format!("„{}“ erschienen", r.console_text) } else { format!("„{}“ nicht innerhalb von {} s", r.console_text, timeout) } });
    }
    // 3: the guest answers on its port (through the isolated network)
    if let Some((host_port, guest_port)) = fwd {
        let mut answered = false;
        let mut detail = String::new();
        while plat::time_us() - start < timeout * 1_000_000 && !o.cancelled() {
            if let Some(a) = plat::net::SocketAddr::parse(&format!("127.0.0.1:{}", host_port)) {
                if let Ok(t) = plat::net::TcpStream::connect(&a, 2000) {
                    t.set_read_timeout(Some(2000));
                    let mut b = [0u8; 256];
                    match t.read(&mut b) {
                        Ok(n) if n > 0 => {
                            answered = true;
                            detail = String::from(String::from_utf8_lossy(&b[..n]).trim());
                            break;
                        }
                        Err(e) if e.kind == plat::net::NetErrorKind::TimedOut => {
                            // connected and kept open: the guest listens
                            answered = true;
                            break;
                        }
                        _ => {}
                    }
                }
            }
            plat::sleep_ms(1000);
        }
        result.checks.push(CheckResult { name: format!("Port {}", guest_port), ok: answered, detail: if answered { if detail.is_empty() { String::from("antwortet") } else { detail } } else { format!("keine Antwort innerhalb von {} s", timeout) } });
    }
    let b = plat::read_file(&live.console).unwrap_or_default();
    result.console = String::from_utf8_lossy(&b[b.len().saturating_sub(4096)..]).into_owned();
    shutdown(&sid, false);
    let all_ok = result.checks.iter().all(|c| c.ok);
    let msg = format!("{} von {} Prüfungen bestanden", result.checks.iter().filter(|c| c.ok).count(), result.checks.len());
    finish_check(ctx, &sid, result, s, if all_ok { Ok(format!("{}: {}", pv.name, msg)) } else { Err(format!("{}: {}", pv.name, msg)) })
}

fn finish_check(ctx: &Ctx, sid: &str, mut r: BootCheckResult, s: &mut Session, res: Result<String, String>) -> Result<String, String> {
    r.finished = now();
    r.ok = res.is_ok();
    r.message = match &res {
        Ok(m) | Err(m) => m.clone(),
    };
    for c in &r.checks {
        engine::session_log(ctx, s, if c.ok { LogLevel::Info } else { LogLevel::Error }, &format!("{}: {} – {}", c.name, if c.ok { "bestanden" } else { "nicht bestanden" }, c.detail));
    }
    let _ = ctx.db.put(C_BOOTCHECK, &r.id, r.to_json());
    set_session(ctx, sid, |x| {
        x.state = String::from("stopped");
        x.stopped = now();
        x.message = r.message.clone();
    });
    res
}

// ------------------------------------------------------------------ start of the server

/// VMs of a former run are gone with it
pub fn start(ctx: &Ctx) {
    for mut s in all::<InstantSession>(&ctx.db, C_INSTANT) {
        if matches!(s.state.as_str(), "starting" | "running" | "migrating" | "migrated") {
            s.state = String::from("interrupted");
            s.stopped = now();
            s.message = String::from("Der Server wurde neu gestartet");
            let _ = ctx.db.put(C_INSTANT, &s.id, s.to_json());
        }
    }
    let base = plat::fs::join(&work(ctx), "instant");
    for e in plat::fs::read_dir(&base).unwrap_or_default() {
        let _ = bc_fs::remove_tree(&plat::fs::join(&base, &e.name));
    }
    // never more than the newest few hundred records
    let mut v: Vec<InstantSession> = all(&ctx.db, C_INSTANT);
    if v.len() > 300 {
        v.sort_by_key(|s| s.started);
        for s in &v[..v.len() - 300] {
            let _ = ctx.db.delete(C_INSTANT, &s.id);
        }
    }
}
