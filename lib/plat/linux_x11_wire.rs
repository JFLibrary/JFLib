// linux_x11_wire -- X11 core protocol over a Unix socket (no Xlib / xcb): connection,
// authentication, setup, request buffer, receive + dispatch, replies, errors
//                                                              [src/plat/linux/x11_wire.s]
//
//   connect() -> bool                ($DISPLAY: ":N[.S]", "unix:N", "/path[:N]"; abstract
//        socket @/tmp/.X11-unix/XN first, then the file socket; TCP "host:N" is refused
//        with a message; MIT-MAGIC-COOKIE-1 from $XAUTHORITY or ~/.Xauthority)
//   req(major | minor << 8, bytes) -> (body offset in out, seq)   bytes include the 4-byte
//        header; BIG-REQUESTS length when > 256 KB; the request space is zeroed
//   out_fd(fd)                       pass fd with the next flush (closed afterwards)
//   flush()                          send everything (waits for the socket, 5 s)
//   recv() -> bytes read (0 = nothing available); fatal on EOF / errors
//   dispatch() -> true when the awaited reply (or its error) arrived; events go to
//        event() (linux_x11_window.rs) in order
//   reply(seq) -> Some(reply bytes) / None (X error)   blocks up to 5 s (then fatal)
//   sync()                           round trip (GetInputFocus)
//   watch(seq) -> watch_error() -> X error code of that request (0 = none)
//   new_id() -> a new resource id
//   poll(events, ms) -> revents      (0 = timeout)
//   change_property(win, prop, type, format, mode, data)   data in bytes
//   get_property(win, prop, type, offset, length, delete) -> reply or None
//   send_event(dest, mask, event32, propagate)
//   fatal(msg)                       "x11: msg", exit(1)
//
// Every X error is recorded; jfgui_X11DEBUG=1 prints the first one of each request type.
// Errors of MIT-SHM requests go to present_error (switch to PutImage).

use alloc::format;
use alloc::vec;
use alloc::vec::Vec;

use rt::os::{syscall3, syscall6};

use super::*;

const SETUP_TIMEOUT: i32 = 5000; // ms
const REPLY_TIMEOUT: u64 = 5000000; // us
const AUTH_MAX: usize = 64;
const MIT_NAME: &[u8] = b"MIT-MAGIC-COOKIE-1";
const AF_UNIX: usize = 1;
const SOCK_STREAM_CLOEXEC: usize = 0x80001;
const MSG_DONTWAIT: usize = 0x40;
const MSG_NOSIGNAL: usize = 0x4000;
const SOL_SOCKET: u32 = 1;
const SCM_RIGHTS: u32 = 1;

// interned atoms, in the order of the A_* indices
const ATOM_NAMES: [&str; A_COUNT] = [
    "WM_PROTOCOLS",
    "WM_DELETE_WINDOW",
    "WM_STATE",
    "WM_CHANGE_STATE",
    "_NET_WM_PING",
    "_NET_WM_PID",
    "_NET_WM_NAME",
    "UTF8_STRING",
    "_NET_WM_STATE",
    "_NET_WM_STATE_MAXIMIZED_VERT",
    "_NET_WM_STATE_MAXIMIZED_HORZ",
    "_NET_WM_STATE_FULLSCREEN",
    "_NET_WM_STATE_HIDDEN",
    "_NET_WM_MOVERESIZE",
    "_NET_WM_WINDOW_TYPE",
    "_NET_WM_WINDOW_TYPE_NORMAL",
    "_MOTIF_WM_HINTS",
    "CLIPBOARD",
    "TARGETS",
    "MULTIPLE",
    "TIMESTAMP",
    "INCR",
    "TEXT",
    "text/plain;charset=utf-8",
    "text/plain",
    "image/png",
    "jfgui_SELECTION",
    "_NET_WM_ICON_NAME",
];

/// "x11: msg" on stderr, exit code 1  [x11_fatal]
pub(super) fn fatal(msg: &str) -> ! {
    eprint(&format!("x11: {msg}\n"));
    crate::exit(1)
}

/// write all bytes (EINTR-safe) -> false on an error  [sys_write_all]
fn write_all(fd: i32, mut d: &[u8]) -> bool {
    while !d.is_empty() {
        // SAFETY: d is a live slice.
        let r = unsafe { syscall3(SYS_WRITE, fd as usize, d.as_ptr() as usize, d.len()) };
        if r == -EINTR {
            continue;
        }
        if r <= 0 {
            return false;
        }
        d = &d[r as usize..];
    }
    true
}

pub(super) fn sys_close(fd: i32) {
    // SAFETY: closes a descriptor we own.
    unsafe { syscall3(SYS_CLOSE, fd as usize, 0, 0) };
}

impl X11 {
    // ------------------------------------------------------------------ connection

    /// x11_connect -> true when connected and set up
    pub(super) fn connect(&mut self) -> bool {
        self.fd = -1;
        if let Some(v) = crate::env("jfgui_X11DEBUG") {
            let c = v.bytes().next().unwrap_or(0);
            if c != 0 && c != b'0' {
                self.debug = true;
            }
        }
        let disp = match crate::env("DISPLAY") {
            Some(d) if !d.is_empty() => d,
            _ => return false,
        };
        let db = disp.as_bytes();
        let mut screen: i32 = 0;
        let mut number: i32 = 0;
        let colon = db.iter().rposition(|&c| c == b':'); // the last ':'
        if db[0] != b'/' {
            match colon {
                None => {
                    if self.debug {
                        eprint(&format!("x11: cannot parse DISPLAY={disp}\n"));
                    }
                    return false;
                }
                Some(0) => {}
                Some(4) if &db[..4] == b"unix" => {}
                Some(_) => {
                    eprint(&format!(
                        "x11: DISPLAY={disp} is a TCP display; only local displays (:N, unix:N, a socket path) are supported\n"
                    ));
                    return false;
                }
            }
        }
        if let Some(c) = colon {
            let rest = &db[c + 1..];
            if rest.first().is_some_and(|d| d.is_ascii_digit()) {
                let (n, used) = parse_int(rest);
                number = n;
                if rest.get(used) == Some(&b'.') {
                    screen = parse_int(&rest[used + 1..]).0;
                }
            }
        }
        if self.inb.is_empty() {
            self.inb = vec![0u8; X11_IN_CAP];
            self.out = Vec::with_capacity(X11_OUT_CAP);
        }
        // host name (WM_CLIENT_MACHINE, Local auth records)
        let mut uts = [0u8; 390 + 2];
        // SAFETY: uts holds a struct utsname (6 x 65 bytes).
        unsafe { syscall3(SYS_UNAME, uts.as_mut_ptr() as usize, 0, 0) };
        let node = &uts[65..130];
        let n = node.iter().position(|&c| c == 0).unwrap_or(65);
        self.nodename = node[..n].to_vec();
        let num = format!("{}", number as u32);
        self.auth_find(num.as_bytes());
        // ---- socket
        let connected = if db[0] == b'/' {
            self.try_sock(db, false)
                || match colon {
                    Some(c) if c + 1 <= 108 => self.try_sock(&db[..c], false), // "/path:N[.S]" -> "/path"
                    _ => false,
                }
        } else {
            let path = format!("/tmp/.X11-unix/X{}", number as u32);
            self.try_sock(path.as_bytes(), true) || self.try_sock(path.as_bytes(), false)
        };
        if !connected {
            if self.debug {
                eprint(&format!("x11: cannot connect to DISPLAY={disp}\n"));
            }
            return false;
        }
        // ---- setup request
        let mut sr = [0u8; 64];
        sr[0] = 0x6C; // 'l': little endian
        sr[2] = 11;
        let mut len = 12;
        if !self.auth.is_empty() {
            let al = self.auth.len();
            sr[6] = MIT_NAME.len() as u8;
            sr[8..10].copy_from_slice(&(al as u16).to_le_bytes());
            sr[12..12 + MIT_NAME.len()].copy_from_slice(MIT_NAME);
            sr[32..32 + al].copy_from_slice(&self.auth);
            len = (al + 32 + 3) & !3;
        }
        if !write_all(self.fd, &sr[..len]) {
            return self.connect_close();
        }
        if !self.read_exact(0, 8) {
            return self.connect_noanswer();
        }
        let more = rd16(&self.inb, 6) as usize * 4; // additional bytes
        if more > X11_IN_CAP - 9 {
            return self.connect_close();
        }
        if !self.read_exact(8, more) {
            return self.connect_noanswer();
        }
        let st = self.inb[0];
        if st != 1 {
            // failed (0) or authenticate (2): print the server's reason
            let rl = if st == 0 { self.inb[1] as usize } else { more };
            let r = &self.inb[8..8 + rl.min(more)];
            let r = &r[..r.iter().position(|&c| c == 0).unwrap_or(r.len())];
            let reason = alloc::string::String::from_utf8_lossy(r).into_owned();
            eprint(&format!("x11: the X server refused the connection: {reason}\n"));
            return self.connect_close();
        }
        if !self.parse_setup(screen as u32) {
            return self.connect_close();
        }
        self.in_len = 0;
        self.in_pos = 0;
        self.seq = 0;
        self.out.clear();
        // non-blocking from now on (recvfrom / sendmsg use MSG_DONTWAIT anyway)
        self.ready = true;
        self.extensions();
        self.intern_atoms();
        true
    }

    fn connect_noanswer(&mut self) -> bool {
        if self.debug {
            eprint("x11: no answer to the connection setup\n");
        }
        self.connect_close()
    }

    fn connect_close(&mut self) -> bool {
        sys_close(self.fd);
        self.fd = -1;
        false
    }

    /// x11_try(path, abstract) -> connected (self.fd)
    fn try_sock(&mut self, path: &[u8], abstr: bool) -> bool {
        let mut sa = [0u8; 112];
        sa[0..2].copy_from_slice(&(AF_UNIX as u16).to_le_bytes());
        let a = abstr as usize;
        // str_copy(sa + 2 + a, path, 106): at most 105 bytes + NUL; longer than 104: fail
        let n = clip_len(path, 106);
        if n > 104 {
            return false;
        }
        sa[2 + a..2 + a + n].copy_from_slice(&path[..n]);
        let alen = if abstr { n + 2 + 1 } else { 110 };
        // SAFETY: socket(AF_UNIX, SOCK_STREAM | SOCK_CLOEXEC, 0).
        let fd = unsafe { syscall3(SYS_SOCKET, AF_UNIX, SOCK_STREAM_CLOEXEC, 0) };
        if fd < 0 {
            return false;
        }
        loop {
            // SAFETY: sa is a sockaddr_un of alen bytes.
            let r = unsafe { syscall3(SYS_CONNECT, fd as usize, sa.as_ptr() as usize, alen) };
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
        if self.debug {
            eprint(&format!(
                "x11: connected to {}{}\n",
                if abstr { "@" } else { "" },
                alloc::string::String::from_utf8_lossy(path)
            ));
        }
        true
    }

    /// x11_read_exact: n bytes into inb[at..] (5 s timeout, EOF, error -> false), during setup
    fn read_exact(&mut self, mut at: usize, mut n: usize) -> bool {
        while n > 0 {
            if self.poll(POLLIN, SETUP_TIMEOUT) & POLLIN == 0 {
                return false;
            }
            // SAFETY: inb[at..at + n] is inside the buffer (n <= X11_IN_CAP - 9 - at).
            let r = unsafe { syscall3(SYS_READ, self.fd as usize, self.inb.as_mut_ptr() as usize + at, n) };
            if r == -EINTR {
                continue;
            }
            if r <= 0 {
                return false;
            }
            at += r as usize;
            n -= r as usize;
        }
        true
    }

    /// x11_parse_setup(screen): the setup reply in inb
    fn parse_setup(&mut self, mut want: u32) -> bool {
        let b = &self.inb;
        // end of the reply: 8 + additional; every walk below is checked against it
        let end = rd16(b, 6) as usize * 4 + 8;
        let ok = (|| -> Option<()> {
            if 40 > end {
                return None;
            }
            let xid_base = rd32(b, 12);
            let xid_mask = rd32(b, 16);
            let maxreq = rd16(b, 26) as u32 * 4;
            let min_kc = b[34] as u32;
            let max_kc = b[35] as u32;
            let screens = b[28] as u32;
            let mut formats = b[29] as usize;
            if b[30] != 0 {
                return None; // image byte order LSBFirst
            }
            let vlen = (rd16(b, 24) as usize + 3) & !3;
            let mut p = vlen + 40; // formats
            if p + formats * 8 > end {
                return None;
            }
            // depth 24 must be stored as 32 bits per pixel
            let mut d24 = false;
            while formats > 0 {
                if b[p] == 24 {
                    d24 = b[p + 1] == 32;
                }
                p += 8;
                formats -= 1;
            }
            if !d24 {
                return None;
            }
            if want >= screens {
                want = 0;
            }
            while want > 0 {
                if p + 40 > end {
                    return None;
                }
                let mut depths = b[p + 39];
                p += 40;
                while depths > 0 {
                    if p + 8 > end {
                        return None;
                    }
                    p += rd16(b, p + 2) as usize * 24 + 8;
                    depths -= 1;
                }
                want -= 1;
            }
            if p + 40 > end {
                return None;
            }
            let root = rd32(b, p);
            let scr_w = rd16(b, p + 20) as u32;
            let scr_h = rd16(b, p + 22) as u32;
            let root_visual = rd32(b, p + 32);
            let root_depth = b[p + 38];
            let mut depths = b[p + 39];
            p += 40;
            let mut found = 0; // 24-bit TrueColor visual found (2 = the root visual)
            let mut visual = 0u32;
            let mut need_cmap = false;
            while depths > 0 {
                if p + 8 > end {
                    return None;
                }
                let depth = b[p];
                let mut nv = rd16(b, p + 2) as usize;
                p += 8;
                if p + nv * 24 > end {
                    return None; // the visuals must fit into the reply
                }
                while nv > 0 {
                    if depth == 24 && b[p + 4] == 4 && rd32(b, p + 8) == 0xFF0000 && rd32(b, p + 12) == 0xFF00 && rd32(b, p + 16) == 0xFF {
                        let id = rd32(b, p);
                        if id == root_visual && root_depth == 24 {
                            found = 2; // the root visual itself: best
                            visual = 0;
                            need_cmap = false;
                        } else if found == 0 {
                            found = 1;
                            visual = id;
                            need_cmap = true;
                        }
                    }
                    p += 24;
                    nv -= 1;
                }
                depths -= 1;
            }
            if found == 0 {
                return None;
            }
            self.xid_base = xid_base;
            self.xid_mask = xid_mask;
            self.xid_inc = xid_mask & xid_mask.wrapping_neg(); // lowest set bit
            self.xid_last = 0;
            self.maxreq = maxreq;
            self.min_kc = min_kc;
            self.max_kc = max_kc;
            self.root = root;
            self.scr_w = scr_w;
            self.scr_h = scr_h;
            self.visual = visual;
            self.need_cmap = need_cmap;
            Some(())
        })();
        if ok.is_none() {
            eprint("x11: unsupported display (needs a 24-bit TrueColor visual stored as 32 bpp, LSBFirst)\n");
            return false;
        }
        if self.debug {
            eprint(&format!(
                "x11: root {:x}, screen {} x {}, visual {:x} (0 = root visual), max request {} bytes\n",
                self.root, self.scr_w, self.scr_h, self.visual, self.maxreq
            ));
        }
        true
    }

    /// x11_extensions: BIG-REQUESTS (enabled), MIT-SHM (version), XKEYBOARD (opcode)
    fn extensions(&mut self) {
        let s_big = self.query_ext_send(b"BIG-REQUESTS");
        let s_shm = self.query_ext_send(b"MIT-SHM");
        let s_xkb = self.query_ext_send(b"XKEYBOARD");
        // the answers, in order
        let mut big_op = 0u32;
        if let Some(r) = self.reply(s_big) {
            if r[8] != 0 {
                big_op = r[9] as u32;
            }
        }
        if let Some(r) = self.reply(s_shm) {
            if r[8] != 0 {
                self.shm_op = r[9] as u32;
                self.shm_ev = r[10] as u32;
            }
        }
        if let Some(r) = self.reply(s_xkb) {
            if r[8] != 0 {
                self.xkb_op = r[9] as u32;
                self.xkb_ev = r[10] as u32;
            }
        }
        if big_op != 0 {
            let (_, seq) = self.req(big_op, 4); // BigReqEnable (minor 0)
            if let Some(r) = self.reply(seq) {
                let words = rd32(&r, 8).min(0x3FFFFFF); // maximum request length in 4-byte units
                self.maxreq = words << 2;
                self.bigreq = true;
            }
        }
        // MIT-SHM version (1.2 = ShmAttachFd)
        if self.shm_op != 0 {
            let (_, seq) = self.req(self.shm_op | XSHM_QUERY_VERSION << 8, 4);
            match self.reply(seq) {
                Some(r) => {
                    let (major, minor) = (rd16(&r, 8), rd16(&r, 10) as u32);
                    if major < 1 {
                        self.shm_op = 0;
                    } else if major > 1 {
                        self.shm_minor = 2;
                    } else {
                        self.shm_minor = minor;
                    }
                }
                None => self.shm_op = 0,
            }
        }
        if self.debug {
            eprint(&format!(
                "x11: BIG-REQUESTS {} (max {} bytes), MIT-SHM op {} v1.{}, XKEYBOARD op {}\n",
                self.bigreq as u32, self.maxreq, self.shm_op, self.shm_minor, self.xkb_op
            ));
        }
    }

    /// QueryExtension(name) -> its sequence number  [x11_query_ext_send]
    pub(super) fn query_ext_send(&mut self, name: &[u8]) -> u32 {
        let (o, seq) = self.req(XR_QUERY_EXTENSION, (name.len() + 8 + 3) & !3);
        wr16(&mut self.out, o, name.len() as u32);
        self.out[o + 4..o + 4 + name.len()].copy_from_slice(name);
        seq
    }

    /// InternAtom(name) (only_if_exists in bit 8 of op) -> its sequence number
    pub(super) fn intern_send(&mut self, name: &[u8], only_if_exists: bool) -> u32 {
        let (o, seq) = self.req(XR_INTERN_ATOM | (only_if_exists as u32) << 8, (name.len() + 8 + 3) & !3);
        wr16(&mut self.out, o, name.len() as u32);
        self.out[o + 4..o + 4 + name.len()].copy_from_slice(name);
        seq
    }

    /// the atom of an InternAtom reply (0 on an error)
    pub(super) fn atom_reply(&mut self, seq: u32) -> u32 {
        self.reply(seq).map_or(0, |r| rd32(&r, 8))
    }

    /// every name of ATOM_NAMES into atoms (one batch)  [x11_intern_atoms]
    fn intern_atoms(&mut self) {
        let mut seqs = [0u32; A_COUNT];
        for (i, n) in ATOM_NAMES.iter().enumerate() {
            seqs[i] = self.intern_send(n.as_bytes(), false);
        }
        for i in 0..A_COUNT {
            self.atoms[i] = self.atom_reply(seqs[i]);
        }
    }

    // ------------------------------------------------------------------ authentication

    /// x11_auth_find(display number): MIT-MAGIC-COOKIE-1 for this display into auth (empty
    /// = connect without authentication).  Records are big endian: u16 family, then address,
    /// number, name, data as (u16 length, bytes).  A record matches with family Wild (65535)
    /// or Local (256) + our host name, and an empty or equal display number; the first match
    /// wins (like libXau).
    fn auth_find(&mut self, num: &[u8]) {
        self.auth.clear();
        let path = match crate::env("XAUTHORITY") {
            Some(p) if !p.is_empty() => p,
            _ => match crate::env("HOME") {
                Some(h) => {
                    let mut p = format!("{h}/.Xauthority");
                    p.truncate(255);
                    p
                }
                None => return,
            },
        };
        let Some(d) = crate::read_file_raw(&path) else { return };
        let end = d.len();
        let mut p = 0usize;
        'rec: loop {
            if p + 2 > end {
                break;
            }
            let family = u16::from_be_bytes([d[p], d[p + 1]]);
            p += 2;
            let mut f = [(0usize, 0usize); 4];
            for fld in f.iter_mut() {
                if p + 2 > end {
                    break 'rec;
                }
                let l = u16::from_be_bytes([d[p], d[p + 1]]) as usize;
                p += 2;
                if p + l > end {
                    break 'rec;
                }
                *fld = (p, l);
                p += l;
            }
            // family / address
            if family != 65535 {
                if family != 256 {
                    continue;
                }
                let (a, al) = f[0];
                if al != self.nodename.len() || d[a..a + al] != self.nodename[..] {
                    continue;
                }
            }
            let (np, nl) = f[1];
            if nl != 0 && (nl != num.len() || &d[np..np + nl] != num) {
                continue;
            }
            let (mp, ml) = f[2];
            if ml != MIT_NAME.len() || &d[mp..mp + ml] != MIT_NAME {
                continue;
            }
            let (dp, dl) = f[3];
            if dl == 0 || dl > AUTH_MAX {
                continue;
            }
            self.auth = d[dp..dp + dl].to_vec();
            break;
        }
        if self.debug {
            eprint(&format!(
                "x11: auth cookie for display {}: {} bytes\n",
                alloc::string::String::from_utf8_lossy(num),
                self.auth.len()
            ));
        }
    }

    // ------------------------------------------------------------------ requests

    /// x11_req(major | minor << 8, bytes) -> (body offset in out, sequence number): bytes
    /// counts the whole request (4-byte header included, multiple of 4); longer than 65535
    /// words: the BIG-REQUESTS form (length 0 + 32-bit length)
    pub(super) fn req(&mut self, op: u32, bytes: usize) -> (usize, u32) {
        let mut ext = 0usize;
        if bytes / 4 > 65535 {
            if !self.bigreq {
                fatal("request too large");
            }
            ext = 4;
        }
        let total = bytes + ext;
        if total > X11_OUT_CAP {
            fatal("request too large");
        }
        if self.out.len() + total > X11_OUT_CAP {
            self.flush();
        }
        let o = self.out.len();
        self.out.resize(o + total, 0);
        wr16(&mut self.out, o, op);
        let body = if ext == 0 {
            wr16(&mut self.out, o + 2, (bytes / 4) as u32);
            o + 4
        } else {
            wr16(&mut self.out, o + 2, 0);
            wr32(&mut self.out, o + 4, (total / 4) as u32);
            o + 8
        };
        self.seq = self.seq.wrapping_add(1);
        (body, self.seq)
    }

    /// x11_out_fd(fd): send fd with the next flush (then closed)
    pub(super) fn out_fd(&mut self, fd: i32) {
        self.out_fd = fd;
        self.out_hasfd = true;
    }

    /// x11_flush: send the request buffer; while the socket is full, keep reading (so
    /// neither side can block on the other) -- fatal after 5 s without progress
    pub(super) fn flush(&mut self) {
        while !self.out.is_empty() {
            if self.fd < 0 {
                self.out.clear();
                return;
            }
            let iov = [self.out.as_ptr() as usize, self.out.len()];
            let mut cmsg = [0u8; 24];
            let mut mh = [0usize; 7]; // msghdr: name, namelen, iov, iovlen, control, controllen, flags
            mh[2] = iov.as_ptr() as usize;
            mh[3] = 1;
            if self.out_hasfd {
                cmsg[0..8].copy_from_slice(&20u64.to_le_bytes()); // cmsg_len = CMSG_LEN(4)
                cmsg[8..12].copy_from_slice(&SOL_SOCKET.to_le_bytes());
                cmsg[12..16].copy_from_slice(&SCM_RIGHTS.to_le_bytes());
                cmsg[16..20].copy_from_slice(&(self.out_fd as u32).to_le_bytes());
                mh[4] = cmsg.as_ptr() as usize;
                mh[5] = 24;
            }
            // SAFETY: mh, iov and cmsg describe live buffers for the call.
            let r = unsafe {
                syscall3(SYS_SENDMSG, self.fd as usize, mh.as_ptr() as usize, MSG_NOSIGNAL | MSG_DONTWAIT)
            };
            if r == -EINTR {
                continue;
            }
            if r == -EAGAIN {
                let ev = self.poll(POLLOUT | POLLIN, SETUP_TIMEOUT);
                if ev == 0 {
                    fatal("the X server stopped reading requests");
                }
                if ev & POLLIN != 0 {
                    self.recv();
                }
                continue;
            }
            if r < 0 {
                fatal("connection to the X server lost while sending");
            }
            if self.out_hasfd {
                self.out_hasfd = false;
                sys_close(self.out_fd);
            }
            self.out.drain(..r as usize);
        }
    }

    /// x11_poll(events, ms) -> revents (0 = timeout); POLLERR / HUP without data are fatal
    /// once the connection is set up
    pub(super) fn poll(&mut self, events: u16, ms: i32) -> u16 {
        loop {
            let mut pfd = [(self.fd as u32 as u64) | (events as u64) << 32];
            // SAFETY: one pollfd.
            let r = unsafe { syscall3(SYS_POLL, pfd.as_mut_ptr() as usize, 1, ms as isize as usize) };
            if r == -EINTR {
                continue;
            }
            if r <= 0 {
                return 0;
            }
            let rev = (pfd[0] >> 48) as u16;
            if rev & POLLIN == 0 && rev & (POLLERR | POLLHUP | POLLNVAL) != 0 && self.ready {
                fatal("connection to the X server lost");
            }
            return rev;
        }
    }

    // ------------------------------------------------------------------ receiving

    /// x11_recv -> bytes received (0 = nothing available); fatal on EOF / errors.  The
    /// consumed front of the buffer is dropped first unless a dispatch is running or a reply
    /// is still being read.
    pub(super) fn recv(&mut self) -> usize {
        if self.in_pos != 0 && self.in_busy == 0 && !self.in_hold {
            self.inb.copy_within(self.in_pos..self.in_len, 0);
            self.in_len -= self.in_pos;
            self.in_pos = 0;
        }
        loop {
            let room = X11_IN_CAP - self.in_len;
            if room == 0 {
                return 0;
            }
            // SAFETY: inb[in_len..] has room bytes.
            let r = unsafe {
                syscall6(SYS_RECVFROM, self.fd as usize, self.inb.as_mut_ptr() as usize + self.in_len, room, MSG_DONTWAIT, 0, 0)
            };
            if r == -EINTR {
                continue;
            }
            if r == -EAGAIN {
                return 0;
            }
            if r < 0 {
                fatal("connection to the X server lost while receiving");
            }
            if r == 0 {
                fatal("the X server closed the connection");
            }
            self.in_len += r as usize;
            return r as usize;
        }
    }

    /// x11_dispatch -> true when the awaited reply or its error arrived (then stops), else
    /// false after handling every complete event in the buffer
    pub(super) fn dispatch(&mut self) -> bool {
        self.in_hold = false;
        self.in_busy += 1;
        let mut res = false;
        loop {
            let pos = self.in_pos;
            let avail = self.in_len - pos;
            if avail < 32 {
                break;
            }
            let typ = self.inb[pos];
            let mut n = 32usize;
            if typ == XE_REPLY || typ & 0x7F == XE_GENERIC {
                let extra = rd32(&self.inb, pos + 4) as usize;
                if extra > (X11_IN_CAP - 32) / 4 {
                    fatal("a reply does not fit into the receive buffer");
                }
                n = extra * 4 + 32;
            }
            if avail < n {
                break; // incomplete
            }
            self.in_pos += n;
            if typ == XE_ERROR {
                let mut pkt = [0u8; 32];
                pkt.copy_from_slice(&self.inb[pos..pos + 32]);
                self.error(&pkt);
                if self.await_on && rd16(&pkt, 2) == self.await_seq {
                    self.await_state = 2;
                    res = true;
                    break;
                }
            } else if typ == XE_REPLY {
                if self.await_on && rd16(&self.inb, pos + 2) == self.await_seq {
                    self.reply_pos = pos;
                    self.await_state = 1;
                    res = true;
                    break;
                }
            } else if n == 32 {
                let mut pkt = [0u8; 32];
                pkt.copy_from_slice(&self.inb[pos..pos + 32]);
                self.event(&mut pkt);
            } else {
                let mut pkt = self.inb[pos..pos + n].to_vec();
                self.event(&mut pkt);
            }
        }
        self.in_busy -= 1;
        res
    }

    /// x11_reply(seq) -> the reply (32 bytes + its extra length), None on an X error
    pub(super) fn reply(&mut self, seq: u32) -> Option<Vec<u8>> {
        if self.await_on {
            fatal("internal error: nested reply wait");
        }
        self.await_seq = seq as u16;
        self.await_on = true;
        self.await_state = 0;
        self.in_hold = false;
        self.flush();
        let deadline = crate::time_us() + REPLY_TIMEOUT;
        loop {
            if self.dispatch() {
                break;
            }
            let now = crate::time_us();
            if now >= deadline {
                fatal("the X server did not answer within 5 s");
            }
            let ms = ((deadline - now) / 1000) as i32 + 1;
            if self.poll(POLLIN, ms) & POLLIN != 0 {
                self.recv();
            }
        }
        self.await_on = false;
        if self.await_state != 1 {
            return None;
        }
        self.in_hold = true;
        let p = self.reply_pos;
        let n = rd32(&self.inb, p + 4) as usize * 4 + 32;
        Some(self.inb[p..p + n].to_vec())
    }

    /// x11_sync: round trip, every earlier request has been processed afterwards
    pub(super) fn sync(&mut self) {
        let (_, seq) = self.req(XR_GET_INPUT_FOCUS, 4);
        self.reply(seq);
    }

    /// x11_watch(seq): remember X errors of this request (read with watch_error)
    pub(super) fn watch(&mut self, seq: u32) {
        self.watch_seq = seq as u16;
        self.watch_err = 0;
        self.watch_on = true;
    }

    /// x11_watch_error -> X error code of the watched request (0 = none); ends the watch
    pub(super) fn watch_error(&mut self) -> u32 {
        self.watch_on = false;
        self.watch_err
    }

    /// x11_error(packet): record an X error; debug output once per request type
    fn error(&mut self, pkt: &[u8; 32]) {
        let code = pkt[1] as u32;
        let major = pkt[10] as u32;
        let minor = rd16(pkt, 8) as u32;
        let seq = rd16(pkt, 2);
        self.err_last = seq as u32;
        if self.watch_on && seq == self.watch_seq {
            self.watch_err = code;
        }
        if major == self.shm_op && major != 0 {
            self.present_error(minor, code);
        }
        if self.debug {
            let (w, bit) = ((major >> 5) as usize, 1u32 << (major & 31));
            if self.err_seen[w] & bit == 0 {
                self.err_seen[w] |= bit;
                eprint(&format!(
                    "x11: X error {} (request {}.{}, sequence {}, value {:x})\n",
                    code,
                    major,
                    minor,
                    seq,
                    rd32(pkt, 4)
                ));
            }
        }
    }

    /// x11_new_id -> a new resource id
    pub(super) fn new_id(&mut self) -> u32 {
        let v = self.xid_last.wrapping_add(self.xid_inc);
        if v > self.xid_mask {
            fatal("out of resource ids");
        }
        self.xid_last = v;
        v | self.xid_base
    }

    // ------------------------------------------------------------------ request helpers

    /// x11_change_property(win, prop, type, format | mode << 8, data, count): data in bytes
    /// (a whole number of format units)
    pub(super) fn change_property(&mut self, win: u32, prop: u32, typ: u32, format: u32, mode: u32, data: &[u8]) {
        let bytes = data.len();
        let count = bytes / (format as usize / 8);
        let (o, _) = self.req(XR_CHANGE_PROPERTY | (mode & 0xFF) << 8, (bytes + 24 + 3) & !3);
        wr32(&mut self.out, o, win);
        wr32(&mut self.out, o + 4, prop);
        wr32(&mut self.out, o + 8, typ);
        wr32(&mut self.out, o + 12, format & 0xFF); // format + 3 unused
        wr32(&mut self.out, o + 16, count as u32);
        self.out[o + 20..o + 20 + bytes].copy_from_slice(data);
    }

    /// a 32-bit property from u32 values (replace)
    pub(super) fn change_prop32(&mut self, win: u32, prop: u32, typ: u32, vals: &[u32]) {
        let mut d = Vec::with_capacity(vals.len() * 4);
        for v in vals {
            d.extend_from_slice(&v.to_le_bytes());
        }
        self.change_property(win, prop, typ, 32, 0, &d);
    }

    /// x11_get_property(win, prop, type, offset, length, delete) -> reply or None
    /// (reply: +1 format, +8 type, +12 bytes after, +16 length in format units, +32 data)
    pub(super) fn get_property(&mut self, win: u32, prop: u32, typ: u32, offset: u32, length: u32, delete: bool) -> Option<Vec<u8>> {
        let (o, seq) = self.req(XR_GET_PROPERTY | (delete as u32) << 8, 24);
        wr32(&mut self.out, o, win);
        wr32(&mut self.out, o + 4, prop);
        wr32(&mut self.out, o + 8, typ);
        wr32(&mut self.out, o + 12, offset);
        wr32(&mut self.out, o + 16, length);
        self.reply(seq)
    }

    /// x11_send_event(dest, mask, event32, propagate)
    pub(super) fn send_event(&mut self, dest: u32, mask: u32, ev: &[u8; 32], propagate: bool) {
        let (o, _) = self.req(XR_SEND_EVENT | (propagate as u32) << 8, 44);
        wr32(&mut self.out, o, dest);
        wr32(&mut self.out, o + 4, mask);
        self.out[o + 8..o + 40].copy_from_slice(ev);
    }
}
