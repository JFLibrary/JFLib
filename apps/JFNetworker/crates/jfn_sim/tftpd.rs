// tftpd -- the TFTP server (RFC 1350, options of RFC 2347, 2348 and 2349): "tftp-server" on UDP
// port 69 serves the files below a root directory of the device's store (files.rs)
//
//   config (JSON, every member may be left out)
//     {"root": "/srv/tftp" ("flash:" on IOS-like devices), "writable": false, "overwrite": true,
//      "port": 69, "timeout_ms": 1000, "retries": 5, "options": true, "blksize_max": 65464,
//      "quota_kb": 0}
//   TftpServer::from_config(&config, ios) / Engine::tftp_server_root(dev)
//
// Every transfer gets a port of its own (RFC 1350 4: the server's transfer ID), bound to the
// address the request came to, so a client talks to it from the second packet on.  Reading: the
// file is read from the disk (OsApi::disk_read), then DATA blocks go out one by one, each
// after the ACK of the one before; a block shorter than the block size ends the file (an empty
// one when its size is a multiple).  A duplicate ACK changes nothing (no second DATA: the
// Sorcerer's Apprentice of RFC 1123 4.2.3.1); a missing ACK sends the block again after the
// timeout, `retries` times, then the transfer is given up.  Writing: the server answers WRQ
// with ACK 0 (or the OACK), takes the DATA blocks in order, writes each to the disk and
// acknowledges it when the write is done; a DATA block that comes again is acknowledged again,
// the last ACK stays ready for a while (the dally of RFC 1350 6) in case the client did not get
// it.  The file lands in the store when its last block is in; a transfer that fails leaves
// nothing behind.
//
// Errors (RFC 1350 5): 1 a file that is not there, 2 a path outside the root, a write to a
// read-only server or to a missing directory, 3 the quota of the root or the device's space,
// 4 a packet that is no request on port 69, 5 a packet from a port of no transfer (it gets the
// error, the transfer goes on), 6 a file that exists when "overwrite" is false, 8 a block
// size the server will not take.  Requests name files relative to the root ("/r1-config" too,
// as tftpd's secure mode); "netascii" converts line ends both ways (tftp.rs).
//
// Options: blksize (8 .. blksize_max), timeout (1 .. 255 s) and tsize (the file's size on a
// read, the coming size on a write, refused with error 3 when it does not fit); an OACK names
// the accepted ones, and a read starts after the client's ACK of block 0.  "options": false
// answers as RFC 1350 alone.
//
// Metrics: reads, writes, done, active, errors, retransmits, timeouts, bytes_sent,
// bytes_received.  Notes (xfertrace.rs) and log lines (tag "tftpd") for each request, error,
// retransmission, timeout and end.

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::any::Any;

use agent_core::jmap::Value;
use jfn_model::files::FsErr;
use jfn_model::msg;
use jfn_model::msg::Msg;
use netpkt::tftp::{self, Packet};
use netpkt::IpAddr;

use crate::http::Wheel;
use crate::kernel::{Engine, Ns, MS, SEC};
use crate::os::{Dgram, Metric, OsApi, Service, SvcEvent};
use crate::trace::Note;
use crate::xfertrace::XNote;

/// the timer key of the deadlines
const K_WHEEL: u32 = 0;
/// syslog severities
const SEV_WARN: u8 = 4;
const SEV_INFO: u8 = 6;

/// what a deadline of a transfer is for
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Due {
    /// the file was read from the disk: the first packet goes out
    Ready,
    /// no answer in time: send the last packet again
    Timeout,
    /// a block is on the disk: its ACK goes out
    Written(u16),
    /// the dally after the last ACK is over
    Dally,
}

/// a running transfer
#[derive(Clone, Debug)]
struct Xfer {
    peer: (IpAddr, u16),
    /// the address the request came to: the transfer's packets leave from it
    local: IpAddr,
    write: bool,
    /// the canonical path and the name as requested
    path: String,
    name: String,
    netascii: bool,
    /// read: what is sent; write: what came so far
    data: Vec<u8>,
    blksize: usize,
    timeout: Ns,
    /// read: the last block sent (0: the OACK); write: the last block acknowledged
    block: u16,
    /// blocks sent or taken (the offset when the numbers roll over)
    blocks: u64,
    /// the last packet sent (to send again)
    last: Vec<u8>,
    tries: u8,
    due: Option<u64>,
    /// write: the last block is in; read: the last block is out
    last_block: bool,
    /// write: a block waits for its write to the disk
    writing: bool,
    /// the options of the OACK (empty: none)
    oack: Vec<(String, String)>,
}

/// the TFTP server
#[derive(Clone, Debug)]
pub struct TftpServer {
    pub port: u16,
    pub root: String,
    pub writable: bool,
    /// a write may replace a file (false: error 6)
    pub overwrite: bool,
    pub timeout: Ns,
    pub retries: u8,
    pub options: bool,
    pub blksize_max: u16,
    /// bytes the files below the root may take (0: only the device's space)
    pub quota: u64,
    xfers: BTreeMap<u16, Xfer>,
    wheel: Wheel<(u16, Due)>,
    /// the root exists (seen at the start and at each request)
    pub root_ok: bool,
    pub reads: u64,
    pub writes: u64,
    pub done: u64,
    pub errors: u64,
    pub retransmits: u64,
    pub timeouts: u64,
    pub bytes_sent: u64,
    pub bytes_received: u64,
}

impl TftpServer {
    pub fn new(ios: bool) -> TftpServer {
        TftpServer {
            port: netpkt::udp::PORT_TFTP,
            root: String::from(if ios { "flash:" } else { "/srv/tftp" }),
            writable: false,
            overwrite: true,
            timeout: SEC,
            retries: 5,
            options: true,
            blksize_max: tftp::BLKSIZE_MAX,
            quota: 0,
            xfers: BTreeMap::new(),
            wheel: Wheel::new(K_WHEEL),
            root_ok: true,
            reads: 0,
            writes: 0,
            done: 0,
            errors: 0,
            retransmits: 0,
            timeouts: 0,
            bytes_sent: 0,
            bytes_received: 0,
        }
    }

    /// the server of a project's definition (svcact.rs)
    pub fn from_config(c: &Value, ios: bool) -> Result<TftpServer, Msg> {
        let mut s = TftpServer::new(ios);
        let num = |k: &str| c.get(k).and_then(|v| v.as_u64());
        let flag = |k: &str| c.get(k).and_then(|v| v.as_bool());
        if let Some(r) = c.get("root") {
            let r = r.as_str().map(str::trim).filter(|r| !r.is_empty()).ok_or_else(|| msg!("\u{201e}root\u{201c} ist kein Verzeichnis."))?;
            s.root = String::from(r);
        }
        if let Some(p) = num("port") {
            s.port = u16::try_from(p).ok().filter(|&p| p > 0).ok_or_else(|| msg!("Port {} gibt es nicht.", p))?;
        }
        s.writable = flag("writable").unwrap_or(false);
        s.overwrite = flag("overwrite").unwrap_or(true);
        s.options = flag("options").unwrap_or(true);
        if let Some(t) = num("timeout_ms") {
            s.timeout = t.clamp(1, 255_000) * MS;
        }
        if let Some(r) = num("retries") {
            s.retries = r.min(50) as u8;
        }
        if let Some(b) = num("blksize_max") {
            s.blksize_max = b.clamp(tftp::BLKSIZE_MIN as u64, tftp::BLKSIZE_MAX as u64) as u16;
        }
        if let Some(q) = c.get("quota_kb").and_then(|v| crate::httpsvc::decimal(v, 1024)) {
            s.quota = q;
        }
        Ok(s)
    }

    /// the root as a canonical path of the device
    fn root_path(&self, os: &OsApi) -> Option<String> {
        os.file_norm(&self.root).ok()
    }

    /// the path a request names, below the root (None: outside it)
    fn resolve(&self, os: &OsApi, name: &str) -> Option<String> {
        let root = self.root_path(os)?;
        let rel = name.trim_start_matches('/');
        // "flash:/" + name on an IOS-like device, "/srv/tftp" + "/" + path on Linux
        let joined = if root.ends_with('/') { format!("{}{}", root, rel) } else { format!("{}/{}", root, rel) };
        let p = os.file_norm(&joined).ok()?;
        let store = os.files()?;
        (store.within(&p, &root) && p != root).then_some(p)
    }

    fn arm(&mut self, os: &mut OsApi) {
        self.wheel.arm(os);
    }

    /// a deadline of a transfer (its earlier one goes)
    fn set_due(&mut self, port: u16, at: Ns, what: Due) {
        let Some(x) = self.xfers.get_mut(&port) else { return };
        if let Some(id) = x.due.take() {
            self.wheel.cancel(id);
        }
        x.due = Some(self.wheel.add(at, (port, what)));
    }

    /// an ERROR from `from_port` (69 or a transfer's) to the peer
    fn error_to(&mut self, os: &mut OsApi, local: IpAddr, from_port: u16, to: (IpAddr, u16), code: u16, text: &str) {
        let p = Packet::Error { code, msg: String::from(text) }.to_bytes();
        os.send_from(local, from_port, to.0, to.1, &p);
    }

    /// a request refused before a transfer began: the error from a port of its own, a note
    fn refuse(&mut self, os: &mut OsApi, d: &Dgram, name: &str, code: u16) {
        self.errors += 1;
        let why = tftp::error_name(code);
        os.note(Note::Xfer(XNote::TftpRefused { file: String::from(name), code, why: String::from(why) }));
        os.log(SEV_WARN, "tftpd", &format!("refused {} from {}: {} ({})", name, d.src, why, code));
        // the error comes from a new port, as a transfer would (RFC 1350 4)
        match os.bind_on(d.dst, 0, None) {
            Ok(p) => {
                self.error_to(os, d.dst, p, (d.src, d.sport), code, why);
                os.close(p);
            }
            Err(_) => self.error_to(os, d.dst, self.port, (d.src, d.sport), code, why),
        }
    }

    /// a request on the server's port
    fn request(&mut self, os: &mut OsApi, d: Dgram) {
        let p = match Packet::parse(&d.data) {
            Ok(p @ (Packet::Rrq { .. } | Packet::Wrq { .. })) => p,
            _ => {
                // anything else on port 69 is no request (an ERROR is not answered)
                if !d.data.starts_with(&[0, tftp::OP_ERROR as u8]) {
                    self.refuse(os, &d, "?", tftp::ERR_ILLEGAL);
                }
                return;
            }
        };
        let (write, name, mode) = match &p {
            Packet::Rrq { file, mode, .. } => (false, file.clone(), mode.clone()),
            Packet::Wrq { file, mode, .. } => (true, file.clone(), mode.clone()),
            _ => return,
        };
        if write {
            self.writes += 1;
        } else {
            self.reads += 1;
        }
        if !tftp::mode_known(&mode) || mode.eq_ignore_ascii_case("mail") {
            return self.refuse(os, &d, &name, tftp::ERR_ILLEGAL);
        }
        let netascii = mode.eq_ignore_ascii_case("netascii");
        self.root_ok = self.root_path(os).is_some_and(|r| os.file_stat(&r).is_some_and(|s| s.dir));
        let Some(path) = self.resolve(os, &name) else {
            return self.refuse(os, &d, &name, tftp::ERR_ACCESS);
        };
        if !self.root_ok {
            return self.refuse(os, &d, &name, tftp::ERR_NOT_FOUND);
        }
        let stat = os.file_stat(&path);
        // the options the server takes (RFC 2347)
        let mut blksize = tftp::BLOCK;
        let mut timeout = self.timeout;
        let mut oack: Vec<(String, String)> = Vec::new();
        let mut data: Vec<u8> = Vec::new();
        if write {
            if !self.writable {
                return self.refuse(os, &d, &name, tftp::ERR_ACCESS);
            }
            match stat {
                Some(s) if s.dir || s.builtin => return self.refuse(os, &d, &name, tftp::ERR_ACCESS),
                Some(_) if !self.overwrite => return self.refuse(os, &d, &name, tftp::ERR_EXISTS),
                _ => {}
            }
            let parent = os.files().map(|f| f.parent(&path)).unwrap_or_default();
            if !os.file_stat(&parent).is_some_and(|s| s.dir) {
                return self.refuse(os, &d, &name, tftp::ERR_ACCESS);
            }
        } else {
            match os.file_read(&path) {
                Ok(b) => data = if netascii { tftp::to_netascii(&b) } else { b },
                Err(FsErr::NotFound) => return self.refuse(os, &d, &name, tftp::ERR_NOT_FOUND),
                Err(_) => return self.refuse(os, &d, &name, tftp::ERR_ACCESS),
            }
        }
        if self.options {
            if let Some(b) = p.opt("blksize").and_then(|v| v.trim().parse::<u64>().ok()) {
                if b < tftp::BLKSIZE_MIN as u64 {
                    return self.refuse(os, &d, &name, tftp::ERR_OPTION);
                }
                let b = b.min(self.blksize_max as u64);
                blksize = b as usize;
                oack.push((String::from("blksize"), format!("{}", b)));
            }
            if let Some(t) = p.opt("timeout").and_then(|v| v.trim().parse::<u64>().ok()).filter(|t| (1..=255).contains(t)) {
                timeout = t * SEC;
                oack.push((String::from("timeout"), format!("{}", t)));
            }
            if let Some(t) = p.opt("tsize").and_then(|v| v.trim().parse::<u64>().ok()) {
                if write {
                    if !self.fits(os, &path, t) {
                        return self.refuse(os, &d, &name, tftp::ERR_DISK_FULL);
                    }
                    oack.push((String::from("tsize"), format!("{}", t)));
                } else {
                    oack.push((String::from("tsize"), format!("{}", data.len())));
                }
            }
        }
        let Ok(port) = os.bind_on(d.dst, 0, None) else {
            return self.refuse(os, &d, &name, tftp::ERR_UNDEFINED);
        };
        os.note(Note::Xfer(XNote::TftpRequest { write, file: name.clone(), from: d.src, blksize: blksize as u16 }));
        os.log(SEV_INFO, "tftpd", &format!("{} from {} for {} ({}-byte blocks)", if write { "WRQ" } else { "RRQ" }, d.src, name, blksize));
        let x = Xfer {
            peer: (d.src, d.sport),
            local: d.dst,
            write,
            path,
            name,
            netascii,
            data,
            blksize,
            timeout,
            block: 0,
            blocks: 0,
            last: Vec::new(),
            tries: 0,
            due: None,
            last_block: false,
            writing: false,
            oack,
        };
        let len = x.data.len() as u64;
        self.xfers.insert(port, x);
        if write {
            self.first_answer(os, port);
        } else {
            // the file comes from the disk first
            let ready = os.disk_read(len);
            self.set_due(port, ready, Due::Ready);
        }
        self.arm(os);
    }

    /// the coming file fits below the root and on the device
    fn fits(&self, os: &OsApi, path: &str, size: u64) -> bool {
        let Some(store) = os.files() else { return false };
        let old = store.stat(path).map_or(0, |s| s.size);
        let more = size.saturating_sub(old);
        let quota_ok = self.quota == 0 || self.root_path(os).is_none_or(|r| store.under(&r).saturating_sub(old) + size <= self.quota);
        quota_ok && more <= store.free()
    }

    /// sends a packet of a transfer and keeps it for sending again
    fn send(&mut self, os: &mut OsApi, port: u16, p: Vec<u8>) {
        let Some(x) = self.xfers.get_mut(&port) else { return };
        let (local, peer, timeout) = (x.local, x.peer, x.timeout);
        os.send_from(local, port, peer.0, peer.1, &p);
        x.last = p;
        let at = os.now() + timeout;
        self.set_due(port, at, Due::Timeout);
    }

    /// the first answer of a transfer: the OACK, ACK 0 or DATA 1
    fn first_answer(&mut self, os: &mut OsApi, port: u16) {
        let Some(x) = self.xfers.get(&port) else { return };
        if !x.oack.is_empty() {
            let opts = x.oack.iter().map(|(k, v)| format!("{}={}", k, v)).collect::<Vec<_>>().join(", ");
            let p = Packet::Oack { opts: x.oack.clone() }.to_bytes();
            os.note(Note::Xfer(XNote::TftpOptions { opts }));
            self.send(os, port, p);
        } else if x.write {
            self.send(os, port, Packet::Ack { block: 0 }.to_bytes());
        } else {
            self.next_block(os, port);
        }
    }

    /// read: the next DATA block
    fn next_block(&mut self, os: &mut OsApi, port: u16) {
        let Some(x) = self.xfers.get_mut(&port) else { return };
        let start = (x.blocks as usize).saturating_mul(x.blksize).min(x.data.len());
        let end = (start + x.blksize).min(x.data.len());
        x.block = x.block.wrapping_add(1);
        x.blocks += 1;
        x.tries = 0;
        x.last_block = end - start < x.blksize;
        let p = Packet::Data { block: x.block, data: x.data[start..end].to_vec() }.to_bytes();
        self.bytes_sent += (end - start) as u64;
        self.send(os, port, p);
    }

    /// the transfer ends (done or not): its port goes
    fn finish(&mut self, os: &mut OsApi, port: u16) {
        if let Some(x) = self.xfers.remove(&port)
            && let Some(id) = x.due
        {
            self.wheel.cancel(id);
        }
        os.close(port);
    }

    /// a packet to a transfer's port
    fn on_xfer(&mut self, os: &mut OsApi, port: u16, d: Dgram) {
        let Some(x) = self.xfers.get(&port) else { return };
        if (d.src, d.sport) != x.peer {
            // another transfer ID: an error to it, this transfer goes on (RFC 1350 4)
            os.note(Note::Xfer(XNote::TftpUnknownTid { from: d.src, port: d.sport }));
            let local = x.local;
            self.error_to(os, local, port, (d.src, d.sport), tftp::ERR_UNKNOWN_TID, tftp::error_name(tftp::ERR_UNKNOWN_TID));
            return;
        }
        let Ok(p) = Packet::parse(&d.data) else { return };
        let write = x.write;
        match p {
            Packet::Error { code, msg } => {
                os.note(Note::Xfer(XNote::TftpError { code, why: msg.clone() }));
                os.log(SEV_WARN, "tftpd", &format!("{} aborted by the client: {} ({})", x.name, msg, code));
                self.errors += 1;
                self.finish(os, port);
            }
            Packet::Ack { block } if !write => self.on_ack(os, port, block),
            Packet::Data { block, data } if write => self.on_data(os, port, block, data),
            _ => {
                let (local, peer) = (x.local, x.peer);
                self.error_to(os, local, port, peer, tftp::ERR_ILLEGAL, tftp::error_name(tftp::ERR_ILLEGAL));
                self.errors += 1;
                self.finish(os, port);
            }
        }
    }

    fn on_ack(&mut self, os: &mut OsApi, port: u16, block: u16) {
        let Some(x) = self.xfers.get(&port) else { return };
        if block != x.block {
            // an ACK of an earlier block (it came late, or twice): nothing goes out again
            os.note(Note::Xfer(XNote::TftpDupAck { block }));
            return;
        }
        if x.last_block {
            let (name, bytes) = (x.name.clone(), x.data.len() as u64);
            os.note(Note::Xfer(XNote::TftpDone { file: name.clone(), write: false, bytes }));
            os.log(SEV_INFO, "tftpd", &format!("sent {} to {}: {} bytes", name, x.peer.0, bytes));
            self.done += 1;
            self.finish(os, port);
            return;
        }
        self.next_block(os, port);
    }

    fn on_data(&mut self, os: &mut OsApi, port: u16, block: u16, data: Vec<u8>) {
        let Some(x) = self.xfers.get(&port) else { return };
        if block == x.block {
            if !x.writing && block != 0 {
                // the client did not get our ACK: it comes again
                let last = x.last_block;
                os.note(Note::Xfer(XNote::TftpDupData { block }));
                let p = Packet::Ack { block }.to_bytes();
                self.send(os, port, p);
                if last {
                    let at = os.now() + 2 * self.timeout;
                    self.set_due(port, at, Due::Dally);
                }
            }
            return;
        }
        if block != x.block.wrapping_add(1) || x.writing || x.last_block {
            return;
        }
        let (path, blksize) = (x.path.clone(), x.blksize);
        let total = x.data.len() as u64 + data.len() as u64;
        if !self.fits(os, &path, total) {
            let x = &self.xfers[&port];
            let (local, peer, name) = (x.local, x.peer, x.name.clone());
            os.note(Note::Xfer(XNote::TftpRefused { file: name.clone(), code: tftp::ERR_DISK_FULL, why: String::from(tftp::error_name(tftp::ERR_DISK_FULL)) }));
            os.log(SEV_WARN, "tftpd", &format!("{} from {} does not fit: {} bytes", name, peer.0, total));
            self.error_to(os, local, port, peer, tftp::ERR_DISK_FULL, tftp::error_name(tftp::ERR_DISK_FULL));
            self.errors += 1;
            self.finish(os, port);
            return;
        }
        let n = data.len();
        let Some(x) = self.xfers.get_mut(&port) else { return };
        x.data.extend_from_slice(&data);
        x.writing = true;
        x.tries = 0;
        if n < blksize {
            x.last_block = true;
        }
        self.bytes_received += n as u64;
        // the block goes to the disk, then its ACK
        let done = os.disk_write(n as u64, 0).unwrap_or_else(|| os.now());
        self.set_due(port, done, Due::Written(block));
    }

    /// a block is on the disk: its ACK, and at the last block the file
    fn written(&mut self, os: &mut OsApi, port: u16, block: u16) {
        let Some(x) = self.xfers.get_mut(&port) else { return };
        x.writing = false;
        x.block = block;
        x.blocks += 1;
        if x.last_block {
            let data = if x.netascii { tftp::from_netascii(&x.data) } else { core::mem::take(&mut x.data) };
            let (path, name, peer) = (x.path.clone(), x.name.clone(), x.peer);
            match os.file_write(&path, &data) {
                Ok(_) => {
                    os.note(Note::Xfer(XNote::TftpDone { file: name.clone(), write: true, bytes: data.len() as u64 }));
                    os.log(SEV_INFO, "tftpd", &format!("received {} from {}: {} bytes", name, peer.0, data.len()));
                    self.done += 1;
                }
                Err(e) => {
                    let code = if e == FsErr::NoSpace { tftp::ERR_DISK_FULL } else { tftp::ERR_ACCESS };
                    let local = self.xfers[&port].local;
                    self.error_to(os, local, port, peer, code, tftp::error_name(code));
                    self.errors += 1;
                    self.finish(os, port);
                    return;
                }
            }
            let p = Packet::Ack { block }.to_bytes();
            self.send(os, port, p);
            // the last ACK stays ready for a retransmitted last block
            let at = os.now() + 2 * self.timeout;
            self.set_due(port, at, Due::Dally);
            return;
        }
        let p = Packet::Ack { block }.to_bytes();
        self.send(os, port, p);
    }

    /// no answer in time: the last packet again, or the end
    fn timed_out(&mut self, os: &mut OsApi, port: u16) {
        let Some(x) = self.xfers.get_mut(&port) else { return };
        if x.tries >= self.retries {
            let (name, block) = (x.name.clone(), x.block);
            self.timeouts += 1;
            os.note(Note::Xfer(XNote::TftpGaveUp { file: name.clone(), block }));
            os.log(SEV_WARN, "tftpd", &format!("timeout on {} at block {}", name, block));
            self.finish(os, port);
            return;
        }
        x.tries += 1;
        let (n, block, p) = (x.tries, x.block, x.last.clone());
        self.retransmits += 1;
        os.note(Note::Xfer(XNote::TftpAgain { block, n }));
        self.send(os, port, p);
    }

    fn deadlines(&mut self, os: &mut OsApi) {
        self.wheel.fired();
        let now = os.now();
        for (id, (port, what)) in self.wheel.due(now) {
            let live = self.xfers.get_mut(&port).is_some_and(|x| {
                let mine = x.due == Some(id);
                if mine {
                    x.due = None;
                }
                mine
            });
            if !live {
                continue;
            }
            match what {
                Due::Ready => self.first_answer(os, port),
                Due::Timeout => self.timed_out(os, port),
                Due::Written(b) => self.written(os, port, b),
                Due::Dally => self.finish(os, port),
            }
        }
        self.arm(os);
    }
}

impl Service for TftpServer {
    fn kind(&self) -> &'static str {
        "tftp-server"
    }
    fn start(&mut self, os: &mut OsApi) {
        self.root_ok = self.root_path(os).is_some_and(|r| os.file_stat(&r).is_some_and(|s| s.dir));
        if os.bind_any(self.port, None).is_err() {
            os.note(Note::PortInUse { port: self.port, svc: String::from("tftp-server") });
        }
    }
    fn on(&mut self, ev: SvcEvent, os: &mut OsApi) {
        match ev {
            SvcEvent::Udp(d) if d.dport == self.port => self.request(os, d),
            SvcEvent::Udp(d) => {
                let port = d.dport;
                self.on_xfer(os, port, d);
                self.arm(os);
            }
            SvcEvent::Timer(K_WHEEL) => self.deadlines(os),
            _ => {}
        }
        // the deadlines as the event left them
        self.arm(os);
    }
    fn stop(&mut self, _os: &mut OsApi) {
        self.xfers.clear();
        self.wheel.clear();
    }
    fn metrics(&self) -> Vec<Metric> {
        alloc::vec![
            Metric::new("reads", self.reads as i64),
            Metric::new("writes", self.writes as i64),
            Metric::new("done", self.done as i64),
            Metric::new("active", self.xfers.len() as i64),
            Metric::new("errors", self.errors as i64),
            Metric::new("retransmits", self.retransmits as i64),
            Metric::new("timeouts", self.timeouts as i64),
            Metric::new("bytes_sent", self.bytes_sent as i64),
            Metric::new("bytes_received", self.bytes_received as i64),
        ]
    }
    fn status(&self) -> String {
        self.status_msg().text()
    }
    fn status_msg(&self) -> Msg {
        if !self.root_ok {
            return msg!("{}: das Verzeichnis fehlt", self.root);
        }
        let how = if self.writable { msg!("lesen und schreiben") } else { msg!("nur lesen") };
        msg!("{} ({}): {} Übertragungen, {} Bytes gesendet, {} empfangen", self.root, how, self.done, self.bytes_sent, self.bytes_received)
    }
    fn clone_box(&self) -> Box<dyn Service> {
        Box::new(self.clone())
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

impl Engine {
    /// the root directories of a device's TFTP servers that are missing (problems.rs)
    pub fn tftp_missing_roots(&self, dev: u32) -> Vec<(String, String)> {
        let Some(ni) = self.node_index(dev) else { return Vec::new() };
        let Some(os) = self.os(ni) else { return Vec::new() };
        os.svcs
            .iter()
            .filter(|s| s.enabled && !s.console)
            .filter_map(|s| {
                let t = s.svc.as_ref()?.as_any().downcast_ref::<TftpServer>()?;
                let ok = os.files.norm(&t.root).ok().is_some_and(|r| os.files.stat(&r).is_some_and(|st| st.dir));
                (!ok).then(|| (s.name.clone(), t.root.clone()))
            })
            .collect()
    }
}
