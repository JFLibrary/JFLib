// bc_vm -- QEMU/KVM for BackupCraft: the QMP client, the VMs a QMP socket shows, consistent
// disk backups by QEMU itself (full, and incremental with persistent dirty bitmaps), the
// allocation of the qcow2 files such a backup leaves, and a read-only NBD server (an instant
// start boots a VM from a restore point through it)
//
//   Qmp::connect(socket, ms) / cmd(name, args) / event(match, ms)
//   reach(socket) -> Up(qmp) | Busy | Down   QEMU serves one QMP client at a time: Busy = another
//                                            program (a backup) holds the socket, Down = no QEMU
//   probe(socket) -> VmInfo                  name, uuid, state, memory, CPUs, disks
//   backup_disks(qmp, disks, bitmap, full, spool) -> dumps   one transaction over all disks
//   Qcow2::open / present / read                             what an incremental dump holds
//   nbd::serve(socket, exports) -> NbdServer                 dropped: stops
//
// A full dump is a raw image of the disk; an incremental one a qcow2 image that holds only
// the clusters of the dirty bitmap (data or zero clusters).  Bitmaps are persistent where the
// disk is qcow2 (they survive a restart of the VM), otherwise they live until QEMU ends; a
// missing bitmap means a full backup.
#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

pub mod nbd;
pub mod qcow2;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use bc_proto::j::{self, Value};
use plat::net::UnixStream;

/// the size of the blocks a disk is stored in (and of the dirty bitmaps)
pub const BLOCK: u64 = 1 << 20;

pub struct Qmp {
    s: UnixStream,
    buf: Vec<u8>,
    events: Vec<Value>,
}

fn obj_with(name: &str, args: Value) -> Value {
    let mut m = j::obj([("execute", Value::string(name))]);
    if args != Value::Null {
        m.insert("arguments", args);
    }
    m
}

impl Qmp {
    /// connect and leave the capabilities negotiation
    pub fn connect(path: &str, timeout_ms: u32) -> Result<Qmp, String> {
        let s = plat::net::connect_unix(path, timeout_ms).map_err(|e| format!("QMP {}: {}", path, e))?;
        s.set_read_timeout(Some(timeout_ms.max(5000)));
        s.set_write_timeout(Some(timeout_ms.max(5000)));
        let mut q = Qmp { s, buf: Vec::new(), events: Vec::new() };
        let greet = q.line()?;
        if greet.get("QMP").is_none() {
            return Err(String::from("kein QMP-Socket"));
        }
        q.cmd("qmp_capabilities", Value::Null)?;
        Ok(q)
    }

    /// the read timeout of later calls
    pub fn set_timeout(&self, ms: u32) {
        self.s.set_read_timeout(Some(ms));
    }

    /// the next message (None: nothing within the read timeout)
    fn read_line(&mut self) -> Result<Option<Value>, String> {
        loop {
            if let Some(i) = self.buf.iter().position(|b| *b == b'\n') {
                let l: Vec<u8> = self.buf.drain(..=i).collect();
                return j::parse(&l).map(Some).ok_or_else(|| String::from("unlesbare QMP-Antwort"));
            }
            let mut b = [0u8; 16384];
            match self.s.read(&mut b) {
                Ok(0) => return Err(String::from("QEMU hat die Verbindung beendet")),
                Ok(n) => {
                    if self.buf.len() > (64 << 20) {
                        return Err(String::from("QMP-Antwort zu groß"));
                    }
                    self.buf.extend_from_slice(&b[..n]);
                }
                Err(e) if e.kind == plat::net::NetErrorKind::TimedOut => return Ok(None),
                Err(e) => return Err(format!("QMP: {}", e)),
            }
        }
    }

    fn line(&mut self) -> Result<Value, String> {
        self.read_line()?.ok_or_else(|| String::from("QEMU antwortet nicht"))
    }

    /// a command -> its "return" value (an "error" reply is the Err)
    pub fn cmd(&mut self, name: &str, args: Value) -> Result<Value, String> {
        let mut t = j::text(&obj_with(name, args));
        t.push('\n');
        self.s.write_all(t.as_bytes()).map_err(|e| format!("QMP: {}", e))?;
        loop {
            let v = self.line()?;
            if v.get("event").is_some() {
                self.events.push(v);
                continue;
            }
            if let Some(r) = v.get("return") {
                return Ok(r.clone());
            }
            if let Some(e) = v.get("error") {
                return Err(format!("{}: {}", name, j::str_of(e, "desc")));
            }
        }
    }

    /// the next event that matches (events seen earlier count first)
    pub fn event(&mut self, want: &dyn Fn(&Value) -> bool, timeout_ms: u32) -> Result<Value, String> {
        if let Some(i) = self.events.iter().position(|e| want(e)) {
            return Ok(self.events.remove(i));
        }
        let start = plat::time_us();
        loop {
            if plat::time_us().saturating_sub(start) > timeout_ms as u64 * 1000 {
                return Err(String::from("QEMU meldet sich nicht rechtzeitig"));
            }
            match self.read_line()? {
                Some(v) if want(&v) => return Ok(v),
                Some(v) => {
                    if v.get("event").is_some() && self.events.len() < 1000 {
                        self.events.push(v);
                    }
                }
                None => {}
            }
        }
    }

    /// the events seen so far (and forget them)
    pub fn take_events(&mut self) -> Vec<Value> {
        core::mem::take(&mut self.events)
    }
}

/// a disk of a VM
#[derive(Clone, Debug, Default)]
pub struct Disk {
    /// the block node (QMP "device" for backups)
    pub node: String,
    /// the image file and its format ("qcow2", "raw", ...)
    pub file: String,
    pub format: String,
    pub size: u64,
    /// "virtio", "ide", "scsi" (how the guest sees it)
    pub bus: String,
}

/// what a QMP socket shows of its VM
#[derive(Clone, Debug, Default)]
pub struct VmInfo {
    pub name: String,
    pub uuid: String,
    /// "running", "paused", ...
    pub status: String,
    pub memory_mb: u64,
    pub cpus: u32,
    pub disks: Vec<Disk>,
    pub version: String,
}

fn bus_of(q: &mut Qmp, qdev: &str) -> String {
    if qdev.is_empty() {
        return String::from("ide");
    }
    let path = qdev.strip_suffix("/virtio-backend").unwrap_or(qdev);
    let ty = q.cmd("qom-get", j::obj([("path", Value::string(path)), ("property", Value::string("type"))])).ok();
    let ty = ty.as_ref().and_then(|v| v.as_str()).unwrap_or("");
    String::from(if ty.starts_with("virtio") || qdev.contains("virtio") {
        "virtio"
    } else if ty.starts_with("scsi") {
        "scsi"
    } else {
        "ide"
    })
}

/// whether QEMU answers on a QMP socket
pub enum Reach {
    Up(Qmp),
    /// connected, but QEMU does not greet: another client holds the socket
    Busy,
    /// no socket, or nothing listens (the VM is off)
    Down,
}

pub fn reach(path: &str, timeout_ms: u32) -> Reach {
    let s = match plat::net::connect_unix(path, timeout_ms) {
        Ok(s) => s,
        Err(_) => return Reach::Down,
    };
    s.set_read_timeout(Some(timeout_ms.max(1000)));
    s.set_write_timeout(Some(timeout_ms.max(1000)));
    let mut q = Qmp { s, buf: Vec::new(), events: Vec::new() };
    match q.read_line() {
        Ok(Some(g)) if g.get("QMP").is_some() => {}
        Ok(None) => return Reach::Busy,
        _ => return Reach::Down,
    }
    match q.cmd("qmp_capabilities", Value::Null) {
        Ok(_) => Reach::Up(q),
        Err(_) => Reach::Busy,
    }
}

/// the VM behind a QMP socket
pub fn probe(path: &str) -> Result<VmInfo, String> {
    let mut q = Qmp::connect(path, 3000)?;
    info(&mut q)
}

pub fn info(q: &mut Qmp) -> Result<VmInfo, String> {
    let name = String::from(j::str_of(&q.cmd("query-name", Value::Null)?, "name"));
    let uuid = String::from(j::str_of(&q.cmd("query-uuid", Value::Null)?, "UUID"));
    let status = String::from(j::str_of(&q.cmd("query-status", Value::Null)?, "status"));
    let memory_mb = j::u64_of(&q.cmd("query-memory-size-summary", Value::Null)?, "base-memory") >> 20;
    let cpus = q.cmd("query-cpus-fast", Value::Null)?.as_array().map_or(1, |a| a.len() as u32);
    let ver = q.cmd("query-version", Value::Null)?;
    let version = ver.get("qemu").map_or(String::new(), |v| format!("{}.{}.{}", j::u64_of(v, "major"), j::u64_of(v, "minor"), j::u64_of(v, "micro")));
    let blocks = q.cmd("query-block", Value::Null)?;
    let mut disks = Vec::new();
    for b in blocks.as_array().unwrap_or(&[]) {
        let Some(ins) = b.get("inserted") else { continue };
        if b.get("removable").and_then(|v| v.as_bool()) == Some(true) {
            continue;
        }
        let image = ins.get("image").cloned().unwrap_or(Value::Null);
        let node = String::from(j::str_of(ins, "node-name"));
        let node = if node.is_empty() { String::from(j::str_of(b, "device")) } else { node };
        let qdev = String::from(j::str_of(b, "qdev"));
        let bus = bus_of(q, &qdev);
        disks.push(Disk {
            node,
            file: String::from(j::str_of(&image, "filename")),
            format: String::from(j::str_of(&image, "format")),
            size: j::u64_of(&image, "virtual-size"),
            bus,
        });
    }
    if name.is_empty() {
        return Err(String::from("Die VM hat keinen Namen (QEMU -name)"));
    }
    Ok(VmInfo { name, uuid, status, memory_mb: memory_mb.max(1), cpus: cpus.max(1), disks, version })
}

/// what a disk backup left in the spool folder
#[derive(Clone, Debug)]
pub struct Dump {
    pub node: String,
    pub size: u64,
    /// the raw image of the whole disk, or (incremental) a qcow2 image of the dirty clusters
    pub path: String,
    pub incremental: bool,
}

fn has_bitmap(q: &mut Qmp, node: &str, bitmap: &str) -> Result<bool, String> {
    let nodes = q.cmd("query-named-block-nodes", j::obj([("flat", Value::Bool(true))]))?;
    for n in nodes.as_array().unwrap_or(&[]) {
        if j::str_of(n, "node-name") != node {
            continue;
        }
        let found = n.get("dirty-bitmaps").and_then(|b| b.as_array()).is_some_and(|a| a.iter().any(|b| j::str_of(b, "name") == bitmap));
        return Ok(found);
    }
    // a drive name (if=...): its bitmaps show in query-block
    let blocks = q.cmd("query-block", Value::Null)?;
    for b in blocks.as_array().unwrap_or(&[]) {
        if j::str_of(b, "device") == node {
            let ins = b.get("inserted");
            return Ok(ins.and_then(|i| i.get("dirty-bitmaps")).and_then(|x| x.as_array()).is_some_and(|a| a.iter().any(|x| j::str_of(x, "name") == bitmap)));
        }
    }
    Ok(false)
}

/// back up the disks of a running VM in one transaction (crash-consistent across its disks):
/// a disk with the bitmap gives an incremental dump unless `full`; the others a full dump and
/// a new bitmap.  Waits for every job; on failure the bitmaps stay as they were.
pub fn backup_disks(q: &mut Qmp, disks: &[Disk], bitmap: &str, full: bool, spool: &str, tag: &str) -> Result<Vec<Dump>, String> {
    let mut actions = Vec::new();
    let mut dumps = Vec::new();
    for (k, d) in disks.iter().enumerate() {
        let present = has_bitmap(q, &d.node, bitmap)?;
        let incremental = present && !full;
        let job = format!("bc-{}-{}", tag, k);
        let path = plat::fs::join(spool, &format!("{}-{}.{}", tag, k, if incremental { "qcow2" } else { "raw" }));
        plat::delete(&path);
        if !incremental {
            let persistent = d.format == "qcow2";
            if present {
                actions.push(j::obj([("type", Value::string("block-dirty-bitmap-clear")),
                                     ("data", j::obj([("node", Value::string(d.node.as_str())), ("name", Value::string(bitmap))]))]));
            } else {
                actions.push(j::obj([("type", Value::string("block-dirty-bitmap-add")),
                                     ("data", j::obj([("node", Value::string(d.node.as_str())), ("name", Value::string(bitmap)),
                                                      ("persistent", Value::Bool(persistent)), ("granularity", Value::u64(BLOCK))]))]));
            }
            actions.push(j::obj([("type", Value::string("drive-backup")),
                                 ("data", j::obj([("device", Value::string(d.node.as_str())), ("job-id", Value::string(job.as_str())),
                                                  ("target", Value::string(path.as_str())), ("format", Value::string("raw")), ("sync", Value::string("full")),
                                                  ("auto-dismiss", Value::Bool(true))]))]));
        } else {
            actions.push(j::obj([("type", Value::string("drive-backup")),
                                 ("data", j::obj([("device", Value::string(d.node.as_str())), ("job-id", Value::string(job.as_str())),
                                                  ("target", Value::string(path.as_str())), ("format", Value::string("qcow2")), ("sync", Value::string("bitmap")),
                                                  ("bitmap", Value::string(bitmap)), ("bitmap-mode", Value::string("on-success")),
                                                  ("auto-dismiss", Value::Bool(true))]))]));
        }
        dumps.push((job, Dump { node: d.node.clone(), size: d.size, path, incremental }));
    }
    // grouped: all jobs succeed or none clears its bitmap (QEMU allows it only for backups alone)
    let only_backups = actions.iter().all(|a| j::str_of(a, "type") == "drive-backup");
    let mut t = j::obj([("actions", Value::Array(actions))]);
    if only_backups && dumps.len() > 1 {
        t.insert("properties", j::obj([("completion-mode", Value::string("grouped"))]));
    }
    q.cmd("transaction", t)?;
    let mut failed = String::new();
    for (job, _) in &dumps {
        let jid = job.clone();
        let ev = q.event(&|v: &Value| {
            matches!(j::str_of(v, "event"), "BLOCK_JOB_COMPLETED" | "BLOCK_JOB_CANCELLED") && v.get("data").is_some_and(|d| j::str_of(d, "device") == jid)
        }, u32::MAX)?;
        let data = ev.get("data").cloned().unwrap_or(Value::Null);
        if j::str_of(&ev, "event") == "BLOCK_JOB_CANCELLED" || !j::str_of(&data, "error").is_empty() {
            failed = format!("Sicherung von {} fehlgeschlagen: {}", job, j::str_of(&data, "error"));
        }
    }
    if !failed.is_empty() {
        for (_, d) in &dumps {
            plat::delete(&d.path);
        }
        return Err(failed);
    }
    Ok(dumps.into_iter().map(|(_, d)| d).collect())
}
