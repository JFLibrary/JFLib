// linux_wl_wire -- Wayland wire protocol: connection, object table, marshalling, receive +
// dispatch, roundtrips, fatal errors and the jfgui_WLDEBUG trace         [src/plat/linux/wl_wire.s]
//
//   Wl::connect() -> bool                ($WAYLAND_DISPLAY in $XDG_RUNTIME_DIR, fallbacks)
//   Wire::send(id, &REQ, args) -> new id (0 if the request creates none)
//   Wire::flush() -> 0 all sent / 1 socket full (the rest stays queued)
//   Wire::flush_wait()                    flush, blocking (5 s, then fatal)
//   Wire::recv() -> bytes read (0 = nothing available); fatal on EOF / errors
//   Wl::dispatch()                        run the handlers of every complete message
//   Wl::roundtrip() -> 0 ok / -1 timeout (5 s)
//   Wire::wait_readable(timeout_ms) -> bool
//   Wire::obj_new(iface) -> id   obj_iface(id) -> (iface, udata, zombie)   set_udata(id, v)
//   Wire::srv_add(id, iface, udata)       a server-created object (data offers, tablets)
//   fatal(msg)                            "wayland: msg", exit(1)
//   wlh_display_error / wlh_delete_id / wlh_sync_done      event handlers
//
// Objects: client ids 1..1023 in a table {iface, zombie, udata}; freed ids (wl_display.
// delete_id) are reused, otherwise ids grow sequentially.  Server ids (>= 0xFF000000) live
// in a small side table.

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use alloc::format;

use rt::os::{syscall3, SYS_CLOSE, SYS_CONNECT, SYS_POLL, SYS_RECVMSG, SYS_SENDMSG, SYS_SOCKET};

use super::proto::{Iface, Req, DISPLAY_SYNC, IFACE_CB_SYNC, IFACE_WL_DISPLAY};
use super::Wl;

const WL_OUT_FLUSH: usize = 4096; // flush first when more than this is queued
const WL_IN_CAP: usize = 65536;
const WL_OUT_FDS: usize = 28; // fds per sendmsg (libwayland's limit)
const WL_IN_FDS: u32 = 64; // receive fd ring (power of two)
const WL_CTRL: usize = 512;
const SRV_MAX: usize = 64;
const STR_MAX: usize = 3000; // longest string we marshal (bytes, without NUL)
const TR_CAP: usize = 1024;
pub(crate) const OBJ_MAX: u32 = 1024; // client object ids 1..1023
pub(crate) const SRV_BASE: u32 = 0xFF00_0000; // first server-allocated id
pub(crate) const MAXARGS: usize = 24;

pub(crate) const EINTR: isize = 4;
pub(crate) const EAGAIN: isize = 11;
const AF_UNIX: usize = 1;
const SOCK_STREAM_CLOEXEC: usize = 0x80001;
const MSG_DONTWAIT: usize = 0x40;
const MSG_NOSIGNAL: usize = 0x4000;
const MSG_CMSG_CLOEXEC: usize = 0x4000_0000;
const MSG_CTRUNC: i32 = 0x08;
const SOL_SOCKET: i32 = 1;
const SCM_RIGHTS: i32 = 1;
pub(crate) const POLLIN: u16 = 0x001;
pub(crate) const POLLOUT: u16 = 0x004;
pub(crate) const POLLERR: u16 = 0x008;
pub(crate) const POLLHUP: u16 = 0x010;
pub(crate) const POLLNVAL: u16 = 0x020;
const SYS_GETUID: usize = 102;

/// struct pollfd
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub(crate) struct PollFd {
    pub fd: i32,
    pub events: u16,
    pub revents: u16,
}

/// poll(fds, timeout ms) -> the raw result (negative errno)
pub(crate) fn poll(fds: &mut [PollFd], ms: i32) -> isize {
    // SAFETY: the slice holds fds.len() pollfd records.
    unsafe { syscall3(SYS_POLL, fds.as_mut_ptr() as usize, fds.len(), ms as isize as usize) }
}

pub(crate) fn sys_close(fd: i32) {
    // SAFETY: closes a descriptor this layer owns.
    unsafe { syscall3(SYS_CLOSE, fd as usize, 0, 0) };
}

#[repr(C)]
struct MsgHdr {
    name: usize,
    namelen: u32,
    _p0: u32,
    iov: usize,
    iovlen: usize,
    control: usize,
    controllen: usize,
    flags: i32,
    _p1: u32,
}

#[repr(C)]
struct IoVec {
    base: usize,
    len: usize,
}

/// one request argument
#[derive(Clone, Copy)]
pub(crate) enum A<'a> {
    U(u32),
    I(i32),
    O(u32),                    // object id (0 = null)
    N(&'static Iface),         // new_id of a typed interface
    B(&'static Iface, u32),    // wl_registry.bind: interface + version, new id
    S(Option<&'a [u8]>),       // string (None = null)
    H(i32),                    // a file descriptor (sent, then closed by flush)
}

/// decoded event arguments: values (i sign-extended, u/f/o/n zero-extended, s 1 = present,
/// h = the fd) and the bytes of strings (without the NUL) and arrays
pub(crate) struct Args<'a> {
    pub v: [u64; MAXARGS],
    pub b: [&'a [u8]; MAXARGS],
}

impl<'a> Args<'a> {
    pub fn u(&self, i: usize) -> u32 {
        self.v[i] as u32
    }
    pub fn i(&self, i: usize) -> i32 {
        self.v[i] as u32 as i32
    }
    /// a string argument (None = null)
    pub fn s(&self, i: usize) -> Option<&'a [u8]> {
        if self.v[i] == 0 { None } else { Some(self.b[i]) }
    }
    /// an array argument's bytes
    pub fn arr(&self, i: usize) -> &'a [u8] {
        self.b[i]
    }
}

#[derive(Clone, Copy, Default)]
struct Obj {
    iface: Option<&'static Iface>,
    zombie: bool,
    udata: u64,
}

#[derive(Clone, Copy)]
struct Srv {
    id: u32,
    iface: &'static Iface,
    udata: u64,
}

pub(crate) struct Wire {
    pub fd: i32,
    pub debug: bool,
    pub quiet: bool, // connect prints nothing (backend probing)
    objs: Vec<Obj>,
    free: Vec<u32>,
    hw: u32,
    srv: Vec<Option<Srv>>,
    out: Vec<u8>,
    out_fds: Vec<i32>,
    pub inb: Vec<u8>,
    in_fds: [i32; WL_IN_FDS as usize],
    fdr: u32,
    fdw: u32,
    in_dispatch: bool,
    sync_id: u32,
    sync_done: bool,
    path: Vec<u8>,
    path_msg: Vec<u8>,
}

/// print "wayland: msg" and exit(1)  [wl_fatal]
pub(crate) fn fatal(msg: &str) -> ! {
    crate::write_stderr(format!("wayland: {msg}\n").as_bytes());
    crate::exit(1)
}

fn rd32(b: &[u8], o: usize) -> u32 {
    match b.get(o..o + 4) {
        Some(s) => u32::from_le_bytes([s[0], s[1], s[2], s[3]]),
        None => 0,
    }
}

/// the bytes up to the first NUL (C string semantics)
pub(crate) fn cstr(b: &[u8]) -> &[u8] {
    match b.iter().position(|&c| c == 0) {
        Some(n) => &b[..n],
        None => b,
    }
}

impl Wire {
    pub fn new() -> Wire {
        let mut objs = vec![Obj::default(); OBJ_MAX as usize];
        objs[1].iface = Some(&IFACE_WL_DISPLAY); // id 1 = wl_display
        Wire {
            fd: -1,
            debug: false,
            quiet: false,
            objs,
            free: Vec::new(),
            hw: 1,
            srv: vec![None; SRV_MAX],
            out: Vec::with_capacity(8192 + 64),
            out_fds: Vec::new(),
            inb: Vec::with_capacity(WL_IN_CAP),
            in_fds: [0; WL_IN_FDS as usize],
            fdr: 0,
            fdw: 0,
            in_dispatch: false,
            sync_id: 0,
            sync_done: false,
            path: Vec::new(),
            path_msg: Vec::new(),
        }
    }

    // ------------------------------------------------------------------ connection

    /// path = dir "/" name -> connected
    fn try_path(&mut self, dir: Option<&[u8]>, name: &[u8]) -> bool {
        let mut p: Vec<u8> = Vec::new();
        if let Some(d) = dir {
            p.extend_from_slice(&d[..d.len().min(199)]);
            while p.last() == Some(&b'/') {
                p.pop(); // strip trailing slashes, then add exactly one
            }
            p.push(b'/');
        }
        p.extend_from_slice(name);
        p.truncate(255);
        self.path = p;
        if self.path.len() > 107 {
            return false;
        }
        let mut addr = [0u8; 112];
        addr[0] = AF_UNIX as u8;
        addr[2..2 + self.path.len()].copy_from_slice(&self.path);
        // SAFETY: socket + connect with a valid sockaddr_un; the descriptor is ours.
        unsafe {
            let fd = syscall3(SYS_SOCKET, AF_UNIX, SOCK_STREAM_CLOEXEC, 0);
            if fd < 0 {
                return false;
            }
            loop {
                let r = syscall3(SYS_CONNECT, fd as usize, addr.as_ptr() as usize, 110);
                if r == -EINTR {
                    continue;
                }
                if r != 0 {
                    sys_close(fd as i32);
                    return false;
                }
                break;
            }
            self.fd = fd as i32;
        }
        true
    }

    /// connect to the compositor -> true (prints why it failed unless quiet)  [wl_connect]
    pub fn connect(&mut self) -> bool {
        self.fd = -1;
        let name = match crate::env("WAYLAND_DISPLAY") {
            Some(v) if !v.is_empty() => v,
            _ => String::from("wayland-0"),
        };
        let name = name.as_bytes();
        let ok = if name[0] == b'/' {
            self.try_path(None, name)
        } else {
            let mut ok = false;
            if let Some(dir) = crate::env("XDG_RUNTIME_DIR") {
                ok = self.try_path(Some(dir.as_bytes()), name);
                if !ok {
                    self.path_msg = self.path.clone(); // keep the first path for the message
                    self.path_msg.truncate(127);
                }
            }
            if !ok {
                // SAFETY: getuid has no arguments.
                let uid = unsafe { syscall3(SYS_GETUID, 0, 0, 0) } as u32;
                let d = format!("/run/user/{uid}");
                ok = self.try_path(Some(d.as_bytes()), name)
                    || self.try_path(Some(b"/mnt/wslg/runtime-dir"), name);
            }
            ok
        };
        if !ok && !self.quiet {
            let p = if self.path_msg.is_empty() { &self.path } else { &self.path_msg };
            let mut m = Vec::new();
            m.extend_from_slice(b"wayland: cannot connect to the compositor at ");
            m.extend_from_slice(p);
            m.extend_from_slice(b" (is WAYLAND_DISPLAY / XDG_RUNTIME_DIR set?)\n");
            crate::write_stderr(&m);
        }
        ok
    }

    // ------------------------------------------------------------------ objects

    /// a new client id for an object of this interface (fatal when the table is full)
    pub fn obj_new(&mut self, iface: &'static Iface) -> u32 {
        let id = match self.free.pop() {
            Some(id) => id,
            None => {
                let id = self.hw + 1;
                if id >= OBJ_MAX {
                    fatal("too many protocol objects");
                }
                self.hw = id;
                id
            }
        };
        self.objs[id as usize] = Obj { iface: Some(iface), zombie: false, udata: 0 };
        id
    }

    /// (interface, udata, zombie) of an object id
    pub fn obj_iface(&self, id: u32) -> Option<(&'static Iface, u64, bool)> {
        if id >= SRV_BASE {
            return self.srv.iter().flatten().find(|s| s.id == id).map(|s| (s.iface, s.udata, false));
        }
        if id >= OBJ_MAX {
            return None;
        }
        let o = &self.objs[id as usize];
        o.iface.map(|i| (i, o.udata, o.zombie))
    }

    /// the udata of an object (client and server objects)
    pub fn set_udata(&mut self, id: u32, v: u64) {
        if id >= SRV_BASE {
            if let Some(s) = self.srv.iter_mut().flatten().find(|s| s.id == id) {
                s.udata = v;
            }
        } else if id < OBJ_MAX {
            self.objs[id as usize].udata = v;
        }
    }

    /// register a server-created object (table full: its events are dropped)
    pub fn srv_add(&mut self, id: u32, iface: &'static Iface, udata: u64) {
        for s in self.srv.iter_mut() {
            if s.is_none() || s.is_some_and(|s| s.id == id) {
                *s = Some(Srv { id, iface, udata });
                return;
            }
        }
    }

    /// a destructor was sent: client objects become zombies until delete_id, server
    /// objects are forgotten at once
    fn obj_gone(&mut self, id: u32) {
        if id >= SRV_BASE {
            if let Some(s) = self.srv.iter_mut().find(|s| s.is_some_and(|s| s.id == id)) {
                *s = None;
            }
        } else if id < OBJ_MAX {
            self.objs[id as usize].zombie = true;
        }
    }

    // ------------------------------------------------------------------ sending

    fn put_u32(&mut self, v: u32) {
        self.out.extend_from_slice(&v.to_le_bytes());
    }

    /// u32 length (incl. NUL) + bytes, 4-padded; None = null string
    fn put_str(&mut self, s: Option<&[u8]>) {
        let Some(s) = s else {
            self.put_u32(0);
            return;
        };
        let mut s = cstr(s);
        if s.len() > STR_MAX {
            let mut n = STR_MAX;
            while n > 0 && s[n] & 0xC0 == 0x80 {
                n -= 1; // utf8_clip: no partial character
            }
            s = &s[..n];
        }
        self.put_u32(s.len() as u32 + 1);
        self.out.extend_from_slice(s);
        let pad = (s.len() + 1 + 3) & !3;
        self.out.resize(self.out.len() + pad - s.len(), 0);
    }

    /// marshal a request -> the new id (0 if the request creates none)  [wl_send]
    pub fn send(&mut self, id: u32, req: &Req, args: &[A]) -> u32 {
        if self.fd < 0 {
            return 0;
        }
        if self.out.len() > WL_OUT_FLUSH || self.out_fds.len() >= WL_OUT_FDS - 4 {
            self.flush_wait();
        }
        let mut new_id = 0;
        let start = self.out.len();
        self.put_u32(id);
        self.put_u32(0);
        for a in args {
            match *a {
                A::U(v) | A::O(v) => self.put_u32(v),
                A::I(v) => self.put_u32(v as u32),
                A::N(iface) => {
                    new_id = self.obj_new(iface);
                    self.put_u32(new_id);
                }
                A::B(iface, ver) => {
                    self.put_str(Some(iface.name.as_bytes()));
                    self.put_u32(ver);
                    new_id = self.obj_new(iface);
                    self.put_u32(new_id);
                }
                A::S(s) => self.put_str(s),
                A::H(fd) => self.out_fds.push(fd),
            }
        }
        let size = (self.out.len() - start) as u32;
        let hdr = size << 16 | req.op as u32;
        self.out[start + 4..start + 8].copy_from_slice(&hdr.to_le_bytes());
        if self.debug {
            let msg = self.out[start..].to_vec();
            self.trace(&msg, req.sig, true);
        }
        if req.destroy {
            self.obj_gone(id);
        }
        new_id
    }

    /// bytes queued for sending
    pub fn pending_out(&self) -> usize {
        self.out.len()
    }

    /// send what is queued -> 0 everything sent, 1 the socket is full (the rest stays
    /// queued)  [wl_flush]
    pub fn flush(&mut self) -> u32 {
        while !self.out.is_empty() {
            let iov = IoVec { base: self.out.as_ptr() as usize, len: self.out.len() };
            let mut ctl = [0u64; 18]; // cmsghdr + WL_OUT_FDS descriptors
            let mut hdr = MsgHdr { name: 0, namelen: 0, _p0: 0, iov: &iov as *const IoVec as usize, iovlen: 1,
                                   control: 0, controllen: 0, flags: 0, _p1: 0 };
            let n = self.out_fds.len().min(WL_OUT_FDS);
            if n != 0 {
                // one SCM_RIGHTS cmsg with every queued fd
                ctl[0] = (n * 4 + 16) as u64;
                ctl[1] = SOL_SOCKET as u64 | (SCM_RIGHTS as u64) << 32;
                for (i, &fd) in self.out_fds.iter().take(n).enumerate() {
                    let w = &mut ctl[2 + i / 2];
                    *w |= (fd as u32 as u64) << (32 * (i % 2));
                }
                hdr.control = ctl.as_ptr() as usize;
                hdr.controllen = (n * 4 + 16 + 7) & !7;
            }
            let r = loop {
                // SAFETY: the header describes the queued bytes and the control block.
                let r = unsafe {
                    syscall3(SYS_SENDMSG, self.fd as usize, &hdr as *const MsgHdr as usize, MSG_NOSIGNAL | MSG_DONTWAIT)
                };
                if r != -EINTR {
                    break r;
                }
            };
            if r == -EAGAIN {
                return 1;
            }
            if r < 0 {
                fatal("connection lost while sending");
            }
            // the fds went with the first byte: close our copies
            for &fd in &self.out_fds {
                sys_close(fd);
            }
            self.out_fds.clear();
            self.out.drain(..r as usize);
        }
        0
    }

    /// flush everything, waiting for the socket (fatal after 5 s)  [wl_flush_wait]
    pub fn flush_wait(&mut self) {
        while self.flush() != 0 {
            let mut p = [PollFd { fd: self.fd, events: POLLOUT, revents: 0 }];
            let r = poll(&mut p, 5000);
            if r == -EINTR || r > 0 {
                continue;
            }
            fatal("the compositor stopped reading requests");
        }
    }

    // ------------------------------------------------------------------ receiving

    /// read what the socket has -> bytes received, 0 if nothing is available (fatal on EOF /
    /// errors)  [wl_recv]
    pub fn recv(&mut self) -> usize {
        if self.inb.capacity() < WL_IN_CAP {
            self.inb.reserve(WL_IN_CAP - self.inb.len());
        }
        loop {
            let len = self.inb.len();
            let room = WL_IN_CAP.saturating_sub(len);
            // SAFETY: the vector has capacity for WL_IN_CAP bytes; we write behind len.
            let iov = IoVec { base: unsafe { self.inb.as_mut_ptr().add(len) } as usize, len: room };
            let mut ctl = [0u64; WL_CTRL / 8];
            let mut hdr = MsgHdr { name: 0, namelen: 0, _p0: 0, iov: &iov as *const IoVec as usize, iovlen: 1,
                                   control: ctl.as_mut_ptr() as usize, controllen: WL_CTRL, flags: 0, _p1: 0 };
            // SAFETY: the header describes the free part of the buffer and the control block.
            let r = unsafe {
                syscall3(SYS_RECVMSG, self.fd as usize, &mut hdr as *mut MsgHdr as usize, MSG_DONTWAIT | MSG_CMSG_CLOEXEC)
            };
            if r == -EINTR {
                continue;
            }
            if r == -EAGAIN {
                return 0;
            }
            if r < 0 {
                fatal("connection lost while receiving");
            }
            if r == 0 {
                fatal("the compositor closed the connection");
            }
            if hdr.flags & MSG_CTRUNC != 0 {
                fatal("file descriptor control data truncated");
            }
            // collect SCM_RIGHTS fds
            let cb: &[u8] = {
                let n = hdr.controllen.min(WL_CTRL);
                // SAFETY: ctl holds WL_CTRL bytes.
                unsafe { core::slice::from_raw_parts(ctl.as_ptr() as *const u8, n) }
            };
            let mut o = 0usize;
            while o + 16 <= cb.len() {
                let clen = u64::from_le_bytes(cb[o..o + 8].try_into().unwrap_or([0; 8])) as usize;
                if clen < 16 {
                    break;
                }
                let lvl = rd32(cb, o + 8) as i32;
                let typ = rd32(cb, o + 12) as i32;
                if lvl == SOL_SOCKET && typ == SCM_RIGHTS {
                    let end = (o + clen).min(cb.len());
                    let mut f = o + 16;
                    while f + 4 <= end {
                        if self.fdw.wrapping_sub(self.fdr) >= WL_IN_FDS {
                            fatal("too many file descriptors received");
                        }
                        self.in_fds[(self.fdw & (WL_IN_FDS - 1)) as usize] = rd32(cb, f) as i32;
                        self.fdw = self.fdw.wrapping_add(1);
                        f += 4;
                    }
                }
                o += (clen + 7) & !7;
            }
            // SAFETY: the kernel wrote r bytes behind len (r <= room).
            unsafe { self.inb.set_len(len + r as usize) };
            return r as usize;
        }
    }

    /// the next received fd (fatal if none arrived)
    fn pop_fd(&mut self) -> i32 {
        if self.fdr == self.fdw {
            fatal("a message expects a file descriptor that did not arrive");
        }
        let fd = self.in_fds[(self.fdr & (WL_IN_FDS - 1)) as usize];
        self.fdr = self.fdr.wrapping_add(1);
        fd
    }

    /// the socket is readable within timeout_ms (-1 = forever)  [wl_wait_readable]
    pub fn wait_readable(&mut self, ms: i32) -> bool {
        loop {
            let mut p = [PollFd { fd: self.fd, events: POLLIN, revents: 0 }];
            let r = poll(&mut p, ms);
            if r == -EINTR {
                continue;
            }
            if r <= 0 {
                return false;
            }
            if p[0].revents & POLLIN != 0 {
                return true;
            }
            fatal("connection lost");
        }
    }

    // ------------------------------------------------------------------ trace (jfgui_WLDEBUG)

    /// print one message ("[wl] -> wl_surface#3.attach(#9, 0, 0)")  [wl_trace]
    pub fn trace(&self, msg: &[u8], sig: &str, outgoing: bool) {
        let mut t: Vec<u8> = Vec::new();
        let id = rd32(msg, 0);
        match self.obj_iface(id) {
            Some((i, _, _)) => {
                t.extend_from_slice(if outgoing { b"[wl] -> " } else { b"[wl] <- " });
                t.extend_from_slice(i.name.as_bytes());
            }
            None => t.extend_from_slice(b"[wl] ?? ?"),
        }
        t.extend_from_slice(format!("#{id}.").as_bytes());
        let (name, args) = match sig.find(':') {
            Some(n) => (&sig[..n], &sig[n + 1..]),
            None => (sig, ""),
        };
        t.extend_from_slice(name.as_bytes());
        t.push(b'(');
        let mut c = 8usize; // message cursor
        let mut first = true;
        for ch in args.bytes() {
            if ch == b'?' {
                continue;
            }
            if !first {
                t.extend_from_slice(b", ");
            }
            first = false;
            if ch == b'h' {
                t.extend_from_slice(b"fd");
                continue;
            }
            let v = rd32(msg, c);
            c += 4;
            match ch {
                b'i' => t.extend_from_slice(format!("{}", v as i32).as_bytes()),
                b'u' => t.extend_from_slice(format!("{v}").as_bytes()),
                b'f' => {
                    let (neg, a) = if (v as i32) < 0 { (true, v.wrapping_neg()) } else { (false, v) };
                    let s = format!("{}{}.{:02}", if neg { "-" } else { "" }, a >> 8, ((a & 255) * 100) >> 8);
                    t.extend_from_slice(s.as_bytes());
                }
                b'o' => {
                    if v == 0 {
                        t.extend_from_slice(b"nil");
                    } else {
                        t.extend_from_slice(format!("#{v}").as_bytes());
                    }
                }
                b'n' => t.extend_from_slice(format!("new #{v}").as_bytes()),
                b'N' => {
                    let s = msg.get(c..).unwrap_or(&[]);
                    t.push(b'"');
                    t.extend_from_slice(cstr(s));
                    t.push(b'"');
                    c += (v as usize + 3) & !3;
                    t.extend_from_slice(format!(" v{} new #{}", rd32(msg, c), rd32(msg, c + 4)).as_bytes());
                    c += 8;
                }
                b's' => {
                    if v == 0 {
                        t.extend_from_slice(b"nil");
                    } else {
                        let s = cstr(msg.get(c..).unwrap_or(&[]));
                        t.push(b'"');
                        t.extend_from_slice(&s[..s.len().min(200)]);
                        t.push(b'"');
                        c += (v as usize + 3) & !3;
                    }
                }
                _ => {
                    // 'a'
                    t.extend_from_slice(format!("[{v} bytes]").as_bytes());
                    c += (v as usize + 3) & !3;
                }
            }
        }
        t.extend_from_slice(b")\n");
        t.truncate(TR_CAP - 1);
        crate::write_stderr(&t);
    }
}

// ------------------------------------------------------------------ dispatch (needs the whole state)

impl Wl {
    /// handle every complete message in the receive buffer  [wl_dispatch]
    pub(crate) fn dispatch(&mut self) {
        if self.w.in_dispatch {
            return;
        }
        self.w.in_dispatch = true;
        let mut buf = core::mem::take(&mut self.w.inb);
        let mut pos = 0usize;
        loop {
            let avail = buf.len() - pos;
            if avail < 8 {
                break;
            }
            let size = u16::from_le_bytes([buf[pos + 6], buf[pos + 7]]) as usize;
            if size < 8 || size & 3 != 0 {
                fatal("malformed message header");
            }
            if avail < size {
                break;
            }
            self.dispatch_one(&buf[pos..pos + size]);
            pos += size;
        }
        buf.drain(..pos);
        let added = core::mem::take(&mut self.w.inb); // received while dispatching
        buf.extend_from_slice(&added);
        self.w.inb = buf;
        self.w.in_dispatch = false;
    }

    /// decode one event and call its handler  [wl_dispatch_one]
    fn dispatch_one(&mut self, msg: &[u8]) {
        let id = rd32(msg, 0);
        let op = u16::from_le_bytes([msg[4], msg[5]]) as usize;
        let Some((iface, udata, zombie)) = self.w.obj_iface(id) else {
            if self.w.debug {
                crate::write_stderr(format!("[wl] <- event for unknown object #{id} (opcode {op}) dropped\n").as_bytes());
            }
            return;
        };
        let Some(ev) = iface.evs.get(op) else { return };
        let sig = match ev.sig.find(':') {
            Some(n) => &ev.sig[n + 1..],
            None => "",
        };
        let mut a = Args { v: [0; MAXARGS], b: [&[]; MAXARGS] };
        let mut fds = [0i32; 8];
        let mut nfd = 0usize;
        let end = msg.len();
        let mut c = 8usize;
        let mut k = 0usize;
        for ch in sig.bytes() {
            if ch == b'?' {
                continue;
            }
            if k >= MAXARGS {
                fatal("malformed event arguments");
            }
            if ch == b'h' {
                let fd = self.w.pop_fd();
                a.v[k] = fd as u32 as u64;
                if nfd < 8 {
                    fds[nfd] = fd;
                    nfd += 1;
                }
                k += 1;
                continue;
            }
            if c + 4 > end {
                fatal("malformed event arguments");
            }
            let v = rd32(msg, c);
            match ch {
                b's' => {
                    c += 4;
                    if v != 0 {
                        let n = v as usize;
                        let next = c + ((n + 3) & !3);
                        if next > end || msg[c + n - 1] != 0 {
                            fatal("malformed event arguments");
                        }
                        a.v[k] = 1;
                        a.b[k] = cstr(&msg[c..c + n]);
                        c = next;
                    }
                }
                b'a' => {
                    let n = v as usize;
                    let next = c + 4 + ((n + 3) & !3);
                    if next > end {
                        fatal("malformed event arguments");
                    }
                    a.v[k] = 1;
                    a.b[k] = &msg[c + 4..c + 4 + n];
                    c = next;
                }
                b'i' => {
                    a.v[k] = v as i32 as i64 as u64;
                    c += 4;
                }
                _ => {
                    a.v[k] = v as u64; // u f o n
                    c += 4;
                }
            }
            k += 1;
        }
        if self.w.debug {
            self.w.trace(msg, ev.sig, false);
        }
        match ev.h {
            Some(h) if !zombie => h(self, id, &a, udata),
            _ => {
                for &fd in &fds[..nfd] {
                    sys_close(fd);
                }
            }
        }
    }

    /// a wl_display.sync and the events up to its answer -> 0 ok / -1 (no answer within
    /// 5 s)  [wl_roundtrip]
    pub(crate) fn roundtrip(&mut self) -> i32 {
        let id = self.w.send(1, &DISPLAY_SYNC, &[A::N(&IFACE_CB_SYNC)]);
        self.w.sync_id = id;
        self.w.sync_done = false;
        let deadline = crate::time_us() + 5_000_000;
        loop {
            self.w.flush_wait();
            self.dispatch();
            if self.w.sync_done {
                return 0;
            }
            let now = crate::time_us();
            if now >= deadline {
                return -1;
            }
            let ms = ((deadline - now) / 1000) as i32 + 1;
            if self.w.wait_readable(ms) {
                self.w.recv();
            }
        }
    }

    /// linuxtest: a handler with decoded arguments (object id 1, udata 0)  [hcall]
    pub(crate) fn test_event(&mut self, iface: &str, event: &str, args: &[u64]) -> bool {
        self.test_event_u(iface, event, args, 0)
    }

    /// (main agent) test_event with the handler's udata  [tcall in plattest pen.s]
    pub(crate) fn test_event_u(&mut self, iface: &str, event: &str, args: &[u64], udata: u64) -> bool {
        let Some(i) = super::proto::ALL_IFACES.iter().find(|i| i.name == iface) else { return false };
        let Some(ev) = i.evs.iter().find(|e| e.sig.split(':').next() == Some(event)) else { return false };
        let mut a = Args { v: [0; MAXARGS], b: [&[]; MAXARGS] };
        for (k, &v) in args.iter().take(MAXARGS).enumerate() {
            a.v[k] = v;
        }
        if let Some(h) = ev.h {
            h(self, 1, &a, udata);
        }
        true
    }
}

// ------------------------------------------------------------------ handlers

/// wl_display.delete_id: the id may be reused  [wlh_delete_id]
pub(crate) fn wlh_delete_id(wl: &mut Wl, _id: u32, a: &Args, _u: u64) {
    let id = a.u(0);
    if !(2..OBJ_MAX).contains(&id) {
        return;
    }
    let o = &mut wl.w.objs[id as usize];
    if o.iface.is_none() {
        return;
    }
    *o = Obj::default();
    wl.w.free.push(id);
}

/// wl_display.error(object, code, message): fatal  [wlh_display_error]
pub(crate) fn wlh_display_error(wl: &mut Wl, _id: u32, a: &Args, _u: u64) {
    let obj = a.u(0);
    let name = wl.w.obj_iface(obj).map_or("?", |(i, _, _)| i.name);
    let mut m = Vec::new();
    m.extend_from_slice(format!("wayland: protocol error on {name}#{obj}, code {}: ", a.u(1)).as_bytes());
    m.extend_from_slice(a.s(2).unwrap_or(b""));
    m.push(b'\n');
    crate::write_stderr(&m);
    crate::exit(1)
}

/// wl_callback.done of a roundtrip  [wlh_sync_done]
pub(crate) fn wlh_sync_done(wl: &mut Wl, id: u32, _a: &Args, _u: u64) {
    if id == wl.w.sync_id {
        wl.w.sync_done = true;
    }
}
