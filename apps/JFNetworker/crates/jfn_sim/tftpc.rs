// tftpc -- the TFTP client of the consoles: busybox-like "tftp" in the Linux shell, "copy ...
// tftp:" at the IOS-like console (cli/files.rs asks for host and file first); a console program
// (a service on the console, os.rs) that ends with the transfer
//
//   TftpClient::get(host, remote, sink, style) / put(host, remote, data, style)
//   Sink    File(path) (the device's store), Running (merged into the running configuration as
//           if typed), Startup (the startup configuration), Stdout (the console)
//   Style   Linux (quiet on success, "tftp: ..." messages and exit status 1 as busybox),
//           Ios (own words: what it does, a '#' per block, a '.' per timeout, the result)
//
// The request goes from a free port to the server's port 69; the server's first answer names
// the transfer ID the client keeps (RFC 1350 4); a packet from any other port gets error 5.
// Lock-step as the server: the next block after the ACK of the one before, the last packet
// again after 1 s without an answer, five times, then the transfer is given up.  -b asks for a
// block size (RFC 2348); an OACK takes it, a server without options answers with DATA 1 or
// ACK 0 and the client goes on with 512 bytes.  A name is looked up first (both families as the
// device can use them).

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::any::Any;

use netpkt::tftp::{self, Packet};
use netpkt::IpAddr;

use crate::kernel::{Ns, MS, SEC};
use crate::os::{Metric, OsApi, Service, SvcEvent};
use crate::resolver::Family;
use crate::trace::Note;
use crate::xfertrace::XNote;

const K_TIMEOUT: u32 = 0;

/// where a file that came goes
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Sink {
    File(String),
    Running,
    Startup,
    Stdout,
}

/// how the client talks on the console
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Style {
    Linux,
    Ios,
}

/// the client
#[derive(Clone, Debug)]
pub struct TftpClient {
    pub host: String,
    pub port: u16,
    pub write: bool,
    pub remote: String,
    /// put: what goes out; get: what came
    pub data: Vec<u8>,
    pub sink: Sink,
    pub style: Style,
    /// the block size to ask for (-b)
    pub blksize: Option<u16>,
    pub timeout: Ns,
    pub retries: u8,
    /// the URL the IOS-like console names in its lines ("tftp://10.0.0.10/r1-config")
    pub url: String,
    addr: Option<IpAddr>,
    lookup: Option<u32>,
    sock: u16,
    tid: Option<u16>,
    block: u16,
    blocks: u64,
    size: usize,
    last: Vec<u8>,
    tries: u8,
    started: Ns,
    last_block: bool,
    /// the request had options: an OACK may answer it
    asked: bool,
    /// no packet of the server yet
    first: bool,
    /// the end: its exit status
    pub exit: Option<i32>,
}

impl TftpClient {
    fn new(host: &str, remote: &str, write: bool, data: Vec<u8>, sink: Sink, style: Style) -> TftpClient {
        TftpClient {
            host: String::from(host),
            port: netpkt::udp::PORT_TFTP,
            write,
            remote: String::from(remote),
            data,
            sink,
            style,
            blksize: None,
            timeout: SEC,
            retries: 5,
            url: format!("tftp://{}/{}", host, remote),
            addr: None,
            lookup: None,
            sock: 0,
            tid: None,
            block: 0,
            blocks: 0,
            size: tftp::BLOCK,
            last: Vec::new(),
            tries: 0,
            started: 0,
            last_block: false,
            asked: false,
            first: true,
            exit: None,
        }
    }
    /// a client that fetches `remote` from `host`
    pub fn get(host: &str, remote: &str, sink: Sink, style: Style) -> TftpClient {
        TftpClient::new(host, remote, false, Vec::new(), sink, style)
    }
    /// a client that sends `data` to `host` as `remote`
    pub fn put(host: &str, remote: &str, data: Vec<u8>, style: Style) -> TftpClient {
        TftpClient::new(host, remote, true, data, Sink::Stdout, style)
    }

    fn fail(&mut self, os: &mut OsApi, linux: &str, ios: &str) {
        match self.style {
            Style::Linux => os.print(&format!("tftp: {}\n", linux)),
            Style::Ios => os.print(&format!("\n%Error {} {} ({})\n", if self.write { "writing" } else { "opening" }, self.url, ios)),
        }
        self.end(os, 1);
    }

    fn end(&mut self, os: &mut OsApi, code: i32) {
        os.cancel(K_TIMEOUT);
        if self.sock != 0 {
            os.close(self.sock);
            self.sock = 0;
        }
        self.exit = Some(code);
        os.exit(code);
    }

    fn begin(&mut self, os: &mut OsApi, addr: IpAddr) {
        self.addr = Some(addr);
        self.sock = match os.bind_any(0, None) {
            Ok(p) => p,
            Err(_) => return self.fail(os, "can't bind a socket", "No free port"),
        };
        let mut opts: Vec<(String, String)> = Vec::new();
        if let Some(b) = self.blksize {
            opts.push((String::from("blksize"), format!("{}", b)));
            opts.push((String::from("tsize"), format!("{}", if self.write { self.data.len() } else { 0 })));
        }
        self.asked = !opts.is_empty();
        let (file, mode) = (self.remote.clone(), String::from("octet"));
        let p = if self.write { Packet::Wrq { file, mode, opts } } else { Packet::Rrq { file, mode, opts } };
        self.started = os.now();
        if self.style == Style::Ios {
            os.print(&if self.write { format!("Sending {} bytes to {} ...\n", self.data.len(), self.url) } else { format!("Fetching {} ...\n", self.url) });
        }
        os.note(Note::Xfer(XNote::TftpAsk { write: self.write, file: self.remote.clone(), to: addr }));
        self.send(os, p.to_bytes(), self.port);
    }

    /// sends a packet to the server (to its transfer port once known) and waits for the answer
    fn send(&mut self, os: &mut OsApi, p: Vec<u8>, port: u16) {
        let Some(addr) = self.addr else { return };
        if !os.send(self.sock, addr, port, &p) {
            return self.fail(os, "network unreachable", "Network is unreachable");
        }
        self.last = p;
        let at = os.now() + self.timeout;
        os.timer(K_TIMEOUT, at);
    }

    fn to_server(&mut self, os: &mut OsApi, p: Vec<u8>) {
        let port = self.tid.unwrap_or(self.port);
        self.send(os, p, port);
    }

    /// put: the next DATA block
    fn next_block(&mut self, os: &mut OsApi) {
        let start = (self.blocks as usize).saturating_mul(self.size).min(self.data.len());
        let end = (start + self.size).min(self.data.len());
        self.block = self.block.wrapping_add(1);
        self.blocks += 1;
        self.tries = 0;
        self.last_block = end - start < self.size;
        let p = Packet::Data { block: self.block, data: self.data[start..end].to_vec() }.to_bytes();
        self.mark(os, "#");
        self.to_server(os, p);
    }

    fn mark(&self, os: &mut OsApi, m: &str) {
        if self.style == Style::Ios {
            os.print(m);
        }
    }

    /// the transfer is complete
    fn complete(&mut self, os: &mut OsApi) {
        os.cancel(K_TIMEOUT);
        let ms = (os.now() - self.started) / MS;
        let bytes = self.data.len();
        os.note(Note::Xfer(XNote::TftpDone { file: self.remote.clone(), write: self.write, bytes: bytes as u64 }));
        if !self.write {
            match self.sink.clone() {
                Sink::File(path) => {
                    if let Err(e) = os.file_write(&path, &self.data) {
                        let (l, i) = (format!("can't open '{}': {}", path, e.errno_text()), e.ios_text());
                        return self.fail(os, &l, i);
                    }
                }
                Sink::Stdout => os.print(&String::from_utf8_lossy(&self.data)),
                Sink::Running | Sink::Startup => {}
            }
        }
        if self.style == Style::Ios {
            let rate = if ms == 0 { bytes as u64 * 1000 } else { bytes as u64 * 1000 / ms };
            os.print(&format!("\nDone: {} bytes in {} ms ({} bytes/s)\n", bytes, ms, rate));
            match self.sink {
                Sink::Running if !self.write => {
                    let text = String::from_utf8_lossy(&self.data).into_owned();
                    let (ni, url) = (os.ni, self.url.clone());
                    os.eng.ios_merge(ni, &text, &url);
                }
                Sink::Startup if !self.write => {
                    let text = String::from_utf8_lossy(&self.data).into_owned();
                    let ni = os.ni;
                    os.eng.ios_set_startup(ni, &text);
                }
                _ => {}
            }
        }
        self.end(os, 0);
    }

    fn on_packet(&mut self, os: &mut OsApi, src: IpAddr, sport: u16, data: &[u8]) {
        if Some(src) != self.addr || self.tid.is_some_and(|t| t != sport) {
            // not this transfer's other end (RFC 1350 4)
            os.note(Note::Xfer(XNote::TftpUnknownTid { from: src, port: sport }));
            let e = Packet::error_of(tftp::ERR_UNKNOWN_TID).to_bytes();
            os.send(self.sock, src, sport, &e);
            return;
        }
        let Ok(p) = Packet::parse(data) else { return };
        let first = self.first;
        self.first = false;
        if self.tid.is_none() {
            self.tid = Some(sport);
        }
        match p {
            Packet::Error { code, msg } => {
                os.note(Note::Xfer(XNote::TftpError { code, why: msg.clone() }));
                let text = if msg.is_empty() { String::from(tftp::error_name(code)) } else { msg };
                let l = format!("server error: ({}) {}", code, text);
                self.fail(os, &l, &text);
            }
            Packet::Oack { opts } if first && self.asked => {
                let o = Packet::Oack { opts: opts.clone() };
                if let Some(b) = o.opt("blksize").and_then(|v| v.parse::<usize>().ok()) {
                    self.size = b.clamp(tftp::BLKSIZE_MIN as usize, tftp::BLKSIZE_MAX as usize);
                }
                let text = opts.iter().map(|(k, v)| format!("{}={}", k, v)).collect::<Vec<_>>().join(", ");
                os.note(Note::Xfer(XNote::TftpOptions { opts: text }));
                if self.write {
                    self.next_block(os);
                } else {
                    self.to_server(os, Packet::Ack { block: 0 }.to_bytes());
                }
            }
            Packet::Data { block, data } if !self.write => {
                if block == self.block.wrapping_add(1) {
                    self.block = block;
                    self.blocks += 1;
                    self.tries = 0;
                    let n = data.len();
                    self.data.extend_from_slice(&data);
                    self.mark(os, "#");
                    let ack = Packet::Ack { block }.to_bytes();
                    let port = sport;
                    if let Some(a) = self.addr {
                        os.send(self.sock, a, port, &ack);
                    }
                    self.last = ack;
                    if n < self.size {
                        return self.complete(os);
                    }
                    let at = os.now() + self.timeout;
                    os.timer(K_TIMEOUT, at);
                } else if block == self.block {
                    // our ACK got lost: again
                    os.note(Note::Xfer(XNote::TftpDupData { block }));
                    let ack = Packet::Ack { block }.to_bytes();
                    os.send(self.sock, src, sport, &ack);
                }
            }
            Packet::Ack { block } if self.write => {
                if block != self.block {
                    os.note(Note::Xfer(XNote::TftpDupAck { block }));
                    return;
                }
                if self.last_block {
                    return self.complete(os);
                }
                self.next_block(os);
            }
            _ => {
                let e = Packet::error_of(tftp::ERR_ILLEGAL).to_bytes();
                os.send(self.sock, src, sport, &e);
                self.fail(os, "illegal packet from the server", "Illegal TFTP operation");
            }
        }
    }

    fn timed_out(&mut self, os: &mut OsApi) {
        if self.tries >= self.retries {
            os.note(Note::Xfer(XNote::TftpGaveUp { file: self.remote.clone(), block: self.block }));
            return self.fail(os, "timeout", "Timed out");
        }
        self.tries += 1;
        os.note(Note::Xfer(XNote::TftpAgain { block: self.block, n: self.tries }));
        self.mark(os, ".");
        let p = self.last.clone();
        self.to_server(os, p);
    }
}

impl Service for TftpClient {
    fn kind(&self) -> &'static str {
        "tftp"
    }
    fn start(&mut self, os: &mut OsApi) {
        match IpAddr::parse(&self.host) {
            Some(a) => self.begin(os, a),
            None => self.lookup = Some(os.resolve_host(&self.host, Family::Any)),
        }
    }
    fn on(&mut self, ev: SvcEvent, os: &mut OsApi) {
        if self.exit.is_some() {
            return;
        }
        match ev {
            SvcEvent::Resolved(id) if Some(id) == self.lookup => {
                let first = os.lookup(id).and_then(|l| l.result.as_ref()).and_then(|r| r.as_ref().ok()).and_then(|a| a.first());
                match first {
                    Some(a) => self.begin(os, a),
                    None => {
                        let l = format!("bad address '{}'", self.host);
                        self.fail(os, &l, "Unknown host");
                    }
                }
            }
            SvcEvent::Udp(d) if d.dport == self.sock => self.on_packet(os, d.src, d.sport, &d.data),
            SvcEvent::Timer(K_TIMEOUT) => self.timed_out(os),
            _ => {}
        }
    }
    fn metrics(&self) -> Vec<Metric> {
        alloc::vec![Metric::new("bytes", self.data.len() as i64), Metric::new("blocks", self.blocks as i64)]
    }
    fn status(&self) -> String {
        format!("tftp {}", self.url)
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
