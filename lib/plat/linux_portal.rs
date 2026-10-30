// linux_portal -- xdg-desktop-portal over a minimal D-Bus client (raw socket, SASL EXTERNAL,
// little-endian marshalling): the file dialog, OpenURI / OpenDirectory, Print
//                                                            [src/plat/linux/portal.s]
//
//   file_dialog(flags, path, exts) -> (1 chosen / 0 cancelled / -1 no portal, index of the
//        chosen extension entry).  Same contract as on Windows (win_filedlg.rs): flags FD_SAVE
//        | index << FD_EXT_SHIFT (FD_FOLDER: OpenFile with "directory", portal version 3),
//        path in / out (UTF-8, '/'; at most PATH_CAP - 1 bytes out),
//        exts ["png=PNG-Bild", "jpg;jpeg=JPEG-Bild"] (an empty entry ends the list).
//   open_url(url) -> 0 / -1         a web page in the default application: OpenURI.OpenURI;
//                                   jfgui_OPEN_DRYRUN=1 only prints it
//   open_folder(path) -> 0 / -1     show a folder / a file in the file manager:
//        OpenURI.OpenDirectory with an O_PATH descriptor (SCM_RIGHTS), fallback
//        OpenURI.OpenURI("file://...").  The Response is awaited for OF_WAIT ms (the window
//        keeps being serviced): code 0 -> 0, 1 / 2 -> -1; without an answer by then the
//        request stays alive on its connection until the next call and 0 is returned.  No
//        bus / no portal / no such path -> -1.  jfgui_OPEN_DRYRUN=1: printed, nothing sent.
//   (crate) portal_print_prepare / portal_print_send / portal_print_end  org.freedesktop.portal
//        .Print for linux_print.rs (PreparePrint with the job's paper / orientation, the page
//        setup of the answer; Print with the PDF as a descriptor)
//
// The dialogs belong to the portal's process.  While one is open the display keeps being
// serviced (wait); input events of that time are dropped, window events (resize, close,
// focus, dpi, paint) are kept and queued again afterwards.
//
// State: the assembly's globals (receive buffer, body / message builders, token, request
// path, ...) are one Bus record (module state, UI thread only).  Messages are byte-identical
// to the assembly's (serials, field order, padding, token "jfgui<n>").

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::cell::RefCell;

use rt::os::syscall6;

use crate::{
    BK_X11, E_CLOSE, E_DPI, E_FOCUS_IN, E_FOCUS_OUT, E_PAINT, E_RESIZE, Event, FD_EXT_SHIFT, FD_FOLDER, FD_SAVE, PAPER_N,
    PJF_LANDSCAPE, PPage, PrintJob,
};

const SYS_READ: usize = 0;
const SYS_WRITE: usize = 1;
const SYS_CLOSE: usize = 3;
const SYS_POLL: usize = 7;
const SYS_SOCKET: usize = 41;
const SYS_CONNECT: usize = 42;
const SYS_SENDMSG: usize = 46;
const SYS_GETUID: usize = 102;
const SYS_OPENAT: usize = 257;
const AF_UNIX: u16 = 1;
const SOCK_STREAM_CLOEXEC: usize = 1 | 0x80000;
const POLLIN: u16 = 1;
const EINTR: isize = -4;
const AT_FDCWD: isize = -100;
const O_PATH: usize = 0x20_0000;
const O_CLOEXEC: usize = 0x8_0000;
const MSG_NOSIGNAL: usize = 0x4000;

const MB_CAP: usize = 32768; // body builder (file filters: every letter as [xX])
const MM_CAP: usize = MB_CAP + 4096; // whole message
const RB_CAP: usize = 65536; // receive buffer
const URI_CAP: usize = 2048;
const UID_CAP: usize = 16;
const HDR_CAP: usize = MM_CAP - MB_CAP - 32; // header fields end below this
const T_HELLO: i32 = 2000; // ms
const T_CALL: i32 = 20000; // ms until the portal answers the call (activation)
const KEEP_MAX: usize = 32; // window events kept while the dialog is open
const MSG_CALL: u8 = 1;
const MSG_RETURN: u8 = 2;
const MSG_ERROR: u8 = 3;
const MSG_SIGNAL: u8 = 4;
const HF_PATH: u8 = 1;
const HF_MEMBER: u8 = 3;
const HF_REPLY: u8 = 5;
const MP_NAME_END: usize = 200; // req_path: the name part ends here (256 bytes total)
const SKIP_DEPTH_MAX: u32 = 32;
const OF_CAP: usize = 1024; // open_folder: absolute path
const OF_WAIT: u32 = 1500; // ms to wait for the portal's Response
const PRN_T_DIALOG: u32 = 3_600_000; // ms the user may spend in the print dialog
const PRN_T_PRINT: u32 = 600_000; // ms until the portal answers Print
/// file_dialog: the path capacity (FD_PATH of ui_filedlg.s)
pub const PATH_CAP: usize = 1040;

const S_BUS_NAME: &[u8] = b"org.freedesktop.DBus";
const S_BUS_PATH: &[u8] = b"/org/freedesktop/DBus";
const S_HELLO: &[u8] = b"Hello";
const S_ADDMATCH: &[u8] = b"AddMatch";
const S_MATCH1: &[u8] = b"type='signal',interface='org.freedesktop.portal.Request',member='Response',path='";
// the portal went away (crashed, restarted by an update): its Response never comes
const S_MATCH_OWNER: &[u8] =
    b"type='signal',sender='org.freedesktop.DBus',member='NameOwnerChanged',arg0='org.freedesktop.portal.Desktop'";
const S_NAME_OWNER_CHANGED: &[u8] = b"NameOwnerChanged";
const S_PORTAL_NAME: &[u8] = b"org.freedesktop.portal.Desktop";
const S_PORTAL_PATH: &[u8] = b"/org/freedesktop/portal/desktop";
const S_FILECHOOSER: &[u8] = b"org.freedesktop.portal.FileChooser";
const S_RESPONSE: &[u8] = b"Response";
const S_OPENURI: &[u8] = b"org.freedesktop.portal.OpenURI";
const S_PRN_IFACE: &[u8] = b"org.freedesktop.portal.Print";
const S_ALL_FILES: &[u8] = "Alle unterstützten Dateien".as_bytes();
const S_HEXD: &[u8] = b"0123456789abcdef";
const S_HEXU: &[u8] = b"0123456789ABCDEF";

// PAPER_*: mm (portrait), PWG name, display name
static PRN_PAPERS: [(f64, f64, &[u8], &[u8]); 5] = [
    (210.0, 297.0, b"iso_a4", b"A4"),
    (215.9, 279.4, b"na_letter", b"US Letter"),
    (148.0, 210.0, b"iso_a5", b"A5"),
    (215.9, 355.6, b"na_legal", b"US Legal"),
    (297.0, 420.0, b"iso_a3", b"A3"),
];
static PRN_DKEYS: [&[u8]; 6] = [b"Width", b"Height", b"MarginTop", b"MarginBottom", b"MarginLeft", b"MarginRight"];
const PRN_MM_PT: f64 = 2.834645669291339; // 72 / 25.4

/// a header field value of the current message (msg_field)
enum Field {
    None,
    Str(usize, usize), // bytes [start, end) of rbuf (NUL at end)
    U32(u32),
}

struct Bus {
    fd: i32,
    rbuf: Vec<u8>, // RB_CAP + 16
    rlen: usize,
    mlen: usize, // current message length (msg_ready)
    mhdr: usize, // its body offset
    rp: usize,   // read position
    body: Vec<u8>,
    b_over: bool, // the body did not fit: msg_send refuses it
    msg: Vec<u8>,
    m_path: &'static [u8],
    m_iface: &'static [u8],
    m_member: &'static [u8],
    m_dest: &'static [u8],
    m_sig: Option<&'static [u8]>,
    m_nfds: u32, // descriptors sent with the next msg_send
    m_sendfd: i32,
    token_n: u32,
    token: Vec<u8>,
    req_path: Vec<u8>,
    handle: Vec<u8>,
    uri: Vec<u8>,
    filter: Vec<u8>,
    name: Vec<u8>,
    folder: Vec<u8>,
    keep: Vec<Event>,
    skip_depth: u32,
    neg_fd: bool, // bus_open negotiates descriptor passing
    fd_ok: bool,  // ... and the bus agreed
    of_keep: i32, // connection kept for a pending open_folder request (0 none)
    prn_open: bool,
    prn_uniq: Vec<u8>,
}

struct Global(RefCell<Option<Bus>>);
// SAFETY: the portal is used from the UI thread only (as the assembly's plain globals);
// RefCell checks the (non-reentrant) borrows at run time.
unsafe impl Sync for Global {}
static BUS: Global = Global(RefCell::new(None));

fn with_bus<R>(f: impl FnOnce(&mut Bus) -> R) -> R {
    let mut g = BUS.0.borrow_mut();
    let b = g.get_or_insert_with(Bus::new);
    f(b)
}

// ---------------------------------------------------------------- string helpers

/// the bytes of a C string: up to the first NUL
fn cstr(s: &[u8]) -> &[u8] {
    s.iter().position(|&c| c == 0).map_or(s, |n| &s[..n])
}

/// str_copy(dst, s, cap): at most cap - 1 bytes, never splitting a UTF-8 sequence
fn copy_cap(s: &[u8], cap: usize) -> Vec<u8> {
    let s = cstr(s);
    if cap == 0 {
        return Vec::new();
    }
    let mut n = s.len();
    if n > cap - 1 {
        n = cap - 1;
        while n > 0 && s[n] & 0xC0 == 0x80 {
            n -= 1;
        }
    }
    s[..n].to_vec()
}

/// str_cat(dst, s, cap)
fn cat_cap(d: &mut Vec<u8>, s: &[u8], cap: usize) {
    if d.len() >= cap {
        return;
    }
    let t = copy_cap(s, cap - d.len());
    d.extend_from_slice(&t);
}

fn sys_close(fd: i32) {
    // SAFETY: closing a descriptor we own.
    unsafe { syscall6(SYS_CLOSE, fd as usize, 0, 0, 0, 0, 0) };
}

fn env_is_1(name: &str) -> bool {
    crate::env(name).is_some_and(|v| v.as_bytes().first() == Some(&b'1'))
}

/// "x11:<XID in hex>" under X11 (the chooser is modal over the app), "" under Wayland
fn parent() -> Vec<u8> {
    if crate::disp_backend() == BK_X11 {
        format!("x11:{:x}", crate::x11s::main_xid()).into_bytes()
    } else {
        Vec::new()
    }
}

impl Bus {
    fn new() -> Bus {
        Bus {
            fd: -1,
            rbuf: vec![0; RB_CAP + 16],
            rlen: 0,
            mlen: 0,
            mhdr: 0,
            rp: 0,
            body: Vec::with_capacity(MB_CAP),
            b_over: false,
            msg: Vec::with_capacity(MM_CAP),
            m_path: S_BUS_PATH,
            m_iface: S_BUS_NAME,
            m_member: S_HELLO,
            m_dest: S_BUS_NAME,
            m_sig: None,
            m_nfds: 0,
            m_sendfd: -1,
            token_n: 0,
            token: Vec::new(),
            req_path: Vec::new(),
            handle: Vec::new(),
            uri: Vec::new(),
            filter: Vec::new(),
            name: Vec::new(),
            folder: Vec::new(),
            keep: Vec::new(),
            skip_depth: 0,
            neg_fd: false,
            fd_ok: false,
            of_keep: 0,
            prn_open: false,
            prn_uniq: Vec::new(),
        }
    }

    // ------------------------------------------------------------ connection

    /// connect and authenticate -> true  [bus_open]
    fn bus_open(&mut self) -> bool {
        self.fd = -1;
        self.rlen = 0;
        let mut sa = [0u8; 112];
        sa[..2].copy_from_slice(&AF_UNIX.to_le_bytes());
        // DBUS_SESSION_BUS_ADDRESS "unix:path=..." / "unix:abstract=...", else /run/user/UID/bus
        let mut alen = 0usize;
        if let Some(a) = crate::env("DBUS_SESSION_BUS_ADDRESS").filter(|a| a.starts_with("unix:")) {
            let b = cstr(a.as_bytes());
            let mut i = 5;
            loop {
                let rest = &b[i.min(b.len())..];
                let (abstr, skip) = if rest.starts_with(b"path=") {
                    (false, 5)
                } else if rest.starts_with(b"abstract=") {
                    (true, 9)
                } else {
                    // skip "key=value," and try the next key
                    match rest.iter().position(|&c| c == b',') {
                        Some(k) => {
                            i += k + 1;
                            continue;
                        }
                        None => break,
                    }
                };
                let mut o = if abstr { 3 } else { 2 };
                for &c in &rest[skip..] {
                    if c == b',' {
                        break;
                    }
                    if o >= 108 {
                        return false;
                    }
                    sa[o] = c;
                    o += 1;
                }
                alen = o;
                break;
            }
        }
        if alen == 0 {
            // SAFETY: getuid has no arguments.
            let uid = unsafe { syscall6(SYS_GETUID, 0, 0, 0, 0, 0, 0) } as u32;
            let p = copy_cap(format!("/run/user/{uid}/bus").as_bytes(), 100);
            sa[2..2 + p.len()].copy_from_slice(&p);
            alen = p.len() + 2;
        }
        // SAFETY: socket / connect with a sockaddr_un of alen bytes.
        let fd = unsafe { syscall6(SYS_SOCKET, AF_UNIX as usize, SOCK_STREAM_CLOEXEC, 0, 0, 0, 0) };
        if fd < 0 {
            return false;
        }
        self.fd = fd as i32;
        // SAFETY: as above.
        if unsafe { syscall6(SYS_CONNECT, fd as usize, sa.as_ptr() as usize, alen, 0, 0, 0) } != 0 {
            self.bus_close();
            return false;
        }
        // SASL: "\0AUTH EXTERNAL <hex of the uid digits>\r\n"
        // SAFETY: getuid has no arguments.
        let uid = unsafe { syscall6(SYS_GETUID, 0, 0, 0, 0, 0, 0) } as u32;
        let digits = copy_cap(format!("{uid}").as_bytes(), UID_CAP);
        let mut line = b"AUTH EXTERNAL ".to_vec();
        for &d in &digits {
            line.push(S_HEXD[(d >> 4) as usize]);
            line.push(S_HEXD[(d & 15) as usize]);
        }
        line.extend_from_slice(b"\r\n");
        if !self.bus_write(&[0]) || !self.bus_write(&line) || !self.read_line() || &self.rbuf[..2] != b"OK" {
            self.bus_close();
            return false;
        }
        self.rlen = 0;
        // descriptor passing for open_folder / Print: NEGOTIATE_UNIX_FD -> AGREE_UNIX_FD
        self.fd_ok = false;
        if self.neg_fd {
            if !self.bus_write(b"NEGOTIATE_UNIX_FD\r\n") || !self.read_line() {
                self.bus_close();
                return false;
            }
            if &self.rbuf[..4] == b"AGRE" {
                self.fd_ok = true;
            }
            self.rlen = 0;
        }
        if !self.bus_write(b"BEGIN\r\n") {
            self.bus_close();
            return false;
        }
        true
    }

    fn bus_close(&mut self) {
        if self.fd >= 0 {
            sys_close(self.fd);
            self.fd = -1;
        }
    }

    /// write all bytes -> false on an error  [bus_write]
    fn bus_write(&mut self, mut b: &[u8]) -> bool {
        while !b.is_empty() {
            // SAFETY: write from a live slice.
            let r = unsafe { syscall6(SYS_WRITE, self.fd as usize, b.as_ptr() as usize, b.len(), 0, 0, 0) };
            if r == EINTR {
                continue;
            }
            if r <= 0 {
                return false;
            }
            b = &b[r as usize..];
        }
        true
    }

    /// write the bytes with descriptor fd attached (SCM_RIGHTS) to the first part  [bus_sendfd]
    fn bus_sendfd(&mut self, b: &[u8], fd: i32) -> bool {
        loop {
            let iov = [b.as_ptr() as usize, b.len()];
            let mut ctl = [0u8; 24];
            ctl[..8].copy_from_slice(&20u64.to_le_bytes()); // cmsg_len = CMSG_LEN(4)
            ctl[8..12].copy_from_slice(&1u32.to_le_bytes()); // SOL_SOCKET
            ctl[12..16].copy_from_slice(&1u32.to_le_bytes()); // SCM_RIGHTS
            ctl[16..20].copy_from_slice(&fd.to_le_bytes());
            // msghdr: name, namelen, iov, iovlen, control, controllen, flags
            let mh = [0usize, 0, iov.as_ptr() as usize, 1, ctl.as_ptr() as usize, 24, 0];
            // SAFETY: a msghdr over live buffers of this frame.
            let r = unsafe { syscall6(SYS_SENDMSG, self.fd as usize, mh.as_ptr() as usize, MSG_NOSIGNAL, 0, 0, 0) };
            if r == EINTR {
                continue;
            }
            if r <= 0 {
                return false;
            }
            // the rest (if any) without the descriptor
            return self.bus_write(&b[r as usize..]);
        }
    }

    /// wait up to timeout_ms for bytes -> 1 arrived, 0 timeout, -1 error / closed  [bus_fill]
    fn bus_fill(&mut self, timeout_ms: i32) -> i32 {
        loop {
            let mut pfd = [0u8; 8];
            pfd[..4].copy_from_slice(&self.fd.to_le_bytes());
            pfd[4..6].copy_from_slice(&POLLIN.to_le_bytes());
            // SAFETY: one pollfd.
            let r = unsafe { syscall6(SYS_POLL, pfd.as_mut_ptr() as usize, 1, timeout_ms as isize as usize, 0, 0, 0) };
            if r == EINTR {
                continue;
            }
            if r < 0 {
                return -1;
            }
            if r == 0 {
                return 0;
            }
            if self.rlen >= RB_CAP {
                return -1;
            }
            let room = RB_CAP - self.rlen;
            // SAFETY: read into the free part of rbuf.
            let n = unsafe {
                syscall6(SYS_READ, self.fd as usize, self.rbuf.as_mut_ptr().add(self.rlen) as usize, room, 0, 0, 0)
            };
            if n == EINTR {
                continue;
            }
            if n <= 0 || n as usize > room {
                return -1;
            }
            self.rlen += n as usize;
            return 1;
        }
    }

    /// rbuf holds a line ending in "\r\n" (SASL)  [read_line]
    fn read_line(&mut self) -> bool {
        loop {
            if self.rlen >= 2 && self.rbuf[..self.rlen].windows(2).any(|w| w == b"\r\n") {
                return true;
            }
            if self.bus_fill(T_HELLO) != 1 {
                return false;
            }
        }
    }

    // ------------------------------------------------------------ writing messages
    // The body never grows past MB_CAP: a write that does not fit sets b_over instead, and
    // msg_send refuses such a message.

    fn body_reset(&mut self) {
        self.body.clear();
        self.b_over = false;
    }

    fn b_align(&mut self, n: usize) {
        while self.body.len() & (n - 1) != 0 {
            if self.body.len() >= MB_CAP {
                self.b_over = true;
                return;
            }
            self.body.push(0);
        }
    }

    fn b_u8(&mut self, v: u8) {
        if self.body.len() >= MB_CAP {
            self.b_over = true;
            return;
        }
        self.body.push(v);
    }

    fn b_u32(&mut self, v: u32) {
        self.b_align(4);
        if self.body.len() + 4 > MB_CAP {
            self.b_over = true;
            return;
        }
        self.body.extend_from_slice(&v.to_le_bytes());
    }

    fn b_f64(&mut self, v: f64) {
        self.b_align(8);
        if self.body.len() + 8 > MB_CAP {
            self.b_over = true;
            return;
        }
        self.body.extend_from_slice(&v.to_le_bytes());
    }

    /// 's' / 'o': u32 length, bytes, NUL
    fn b_str(&mut self, s: &[u8]) {
        let s = cstr(s);
        self.b_u32(s.len() as u32);
        self.b_raw0(s);
    }

    /// 'g': u8 length, bytes, NUL
    fn b_sig(&mut self, s: &[u8]) {
        let s = cstr(s);
        self.b_u8(s.len() as u8);
        self.b_raw0(s);
    }

    // the bytes and a NUL (nothing when they do not fit)
    fn b_raw0(&mut self, s: &[u8]) {
        if self.body.len() + s.len() + 1 > MB_CAP {
            self.b_over = true;
            return;
        }
        self.body.extend_from_slice(s);
        self.body.push(0);
    }

    fn b_bytes(&mut self, s: &[u8]) {
        if self.body.len() + s.len() > MB_CAP {
            self.b_over = true;
            return;
        }
        self.body.extend_from_slice(s);
    }

    /// -> (length slot, first element offset)
    fn b_array_begin(&mut self, elem_align: usize) -> (usize, usize) {
        self.b_u32(0);
        let slot = self.body.len().wrapping_sub(4);
        self.b_align(elem_align);
        (slot, self.body.len())
    }

    /// patch the byte length (a slot past the body: nothing)
    fn b_array_end(&mut self, (slot, start): (usize, usize)) {
        let n = self.body.len().wrapping_sub(start) as u32;
        if slot <= MB_CAP - 4 && slot + 4 <= self.body.len() {
            self.body[slot..slot + 4].copy_from_slice(&n.to_le_bytes());
        }
    }

    /// start an a{sv} entry: align 8, key, variant signature
    fn b_dict_key(&mut self, key: &[u8], sig: &[u8]) {
        self.b_align(8);
        self.b_str(key);
        self.b_sig(sig);
    }

    /// one header field (a field that would end past HDR_CAP marks the message as not
    /// sendable)  [h_field]
    fn h_field(&mut self, value: Option<&[u8]>, code: u8, typ: u8) {
        let Some(v) = value else { return };
        let v = cstr(v);
        if self.msg.len() + v.len() + 24 > HDR_CAP {
            self.b_over = true;
            return;
        }
        while self.msg.len() & 7 != 0 {
            self.msg.push(0);
        }
        self.msg.extend_from_slice(&[code, 1, typ, 0]);
        if typ == b'g' {
            self.msg.push(v.len() as u8);
        } else {
            while self.msg.len() & 3 != 0 {
                self.msg.push(0);
            }
            self.msg.extend_from_slice(&(v.len() as u32).to_le_bytes());
        }
        self.msg.extend_from_slice(v);
        self.msg.push(0);
    }

    /// a method call from the m_* fields and the body -> true when written  [msg_send]
    fn msg_send(&mut self, serial: u32) -> bool {
        if self.b_over {
            return false;
        }
        let mut m = core::mem::take(&mut self.msg);
        m.clear();
        m.extend_from_slice(&[b'l', MSG_CALL, 0, 1]);
        m.extend_from_slice(&(self.body.len() as u32).to_le_bytes());
        m.extend_from_slice(&serial.to_le_bytes());
        m.extend_from_slice(&[0; 4]); // fields length
        self.msg = m;
        self.h_field(Some(self.m_path), HF_PATH, b'o');
        self.h_field(Some(self.m_iface), 2, b's');
        self.h_field(Some(self.m_member), HF_MEMBER, b's');
        self.h_field(Some(self.m_dest), 6, b's');
        self.h_field(self.m_sig, 8, b'g');
        // UNIX_FDS (9, 'u') when a descriptor goes with the message
        if self.m_nfds != 0 {
            while self.msg.len() & 7 != 0 {
                self.msg.push(0);
            }
            self.msg.extend_from_slice(&[9, 1, b'u', 0]);
            self.msg.extend_from_slice(&self.m_nfds.to_le_bytes());
        }
        if self.b_over {
            return false; // a header field did not fit
        }
        let fl = (self.msg.len() - 16) as u32;
        self.msg[12..16].copy_from_slice(&fl.to_le_bytes());
        while self.msg.len() & 7 != 0 {
            self.msg.push(0);
        }
        let mut m = core::mem::take(&mut self.msg);
        m.extend_from_slice(&self.body);
        let ok = if self.m_nfds != 0 { self.bus_sendfd(&m, self.m_sendfd) } else { self.bus_write(&m) };
        self.msg = m;
        ok
    }

    // ------------------------------------------------------------ reading messages

    fn rd32(&self, o: usize) -> u32 {
        u32::from_le_bytes([self.rbuf[o], self.rbuf[o + 1], self.rbuf[o + 2], self.rbuf[o + 3]])
    }

    /// rbuf starts with a complete message (mlen, mhdr set; the lengths come from the peer:
    /// one that cannot fit reads as RB_CAP + 1 long)  [msg_ready]
    fn msg_ready(&mut self) -> bool {
        if self.rlen < 16 {
            return false;
        }
        let hdr = (self.rd32(12) as u64 + 16 + 7) & !7;
        let len = self.rd32(4) as u64 + hdr;
        self.mhdr = hdr.min(RB_CAP as u64 + 1) as usize;
        self.mlen = len.min(RB_CAP as u64 + 1) as usize;
        self.mlen <= self.rlen
    }

    /// -> 1 message ready, 0 timeout, -1 error  [msg_next]
    fn msg_next(&mut self, timeout_ms: i32) -> i32 {
        loop {
            if self.msg_ready() {
                return 1;
            }
            if self.mlen > RB_CAP {
                return -1;
            }
            let r = self.bus_fill(timeout_ms);
            if r != 1 {
                return r;
            }
        }
    }

    /// drop the current message from the receive buffer  [msg_consume]
    fn msg_consume(&mut self) {
        let n = self.mlen.min(self.rlen);
        self.rbuf.copy_within(n..self.rlen, 0);
        self.rlen -= n;
    }

    fn msg_type(&self) -> u8 {
        self.rbuf[1]
    }

    /// the value of header field code of the current message  [msg_field]
    fn msg_field(&self, code: u8) -> Field {
        let b = &self.rbuf;
        let mut p: u64 = 16;
        let end = (self.rd32(12) as u64 + 16).min(self.mhdr as u64);
        loop {
            p = (p + 7) & !7 & 0xFFFF_FFFF;
            if p >= end {
                return Field::None;
            }
            let c = b[p as usize];
            let slen = b[p as usize + 1] as u64;
            let typ = b[p as usize + 2];
            p = (p + slen + 3) & 0xFFFF_FFFF;
            match typ {
                b'g' => {
                    if p >= end {
                        return Field::None;
                    }
                    let n = b[p as usize] as u64;
                    p += 1;
                    let e = p + n;
                    if e >= end || b[e as usize] != 0 {
                        return Field::None;
                    }
                    if c == code {
                        return Field::Str(p as usize, e as usize);
                    }
                    p = e + 1;
                }
                b'u' => {
                    p = (p + 3) & !3;
                    if p + 4 > end {
                        return Field::None;
                    }
                    let v = self.rd32(p as usize);
                    p += 4;
                    if c == code {
                        return Field::U32(v);
                    }
                }
                _ => {
                    p = (p + 3) & !3;
                    if p + 4 > end {
                        return Field::None;
                    }
                    let n = self.rd32(p as usize) as u64;
                    p += 4;
                    let e = p + n;
                    if e >= end || b[e as usize] != 0 {
                        return Field::None;
                    }
                    if c == code {
                        return Field::Str(p as usize, e as usize);
                    }
                    p = (e + 1) & 0xFFFF_FFFF;
                }
            }
        }
    }

    /// a string field equals s
    fn field_is(&self, code: u8, s: &[u8]) -> bool {
        match self.msg_field(code) {
            Field::Str(a, e) => &self.rbuf[a..e] == cstr(s),
            _ => false,
        }
    }

    /// the reply / error to serial is at rbuf -> true  [wait_reply]
    fn wait_reply(&mut self, serial: u32, timeout_ms: i32) -> bool {
        loop {
            if self.msg_next(timeout_ms) != 1 {
                return false;
            }
            let t = self.msg_type();
            if (t == MSG_RETURN || t == MSG_ERROR) && matches!(self.msg_field(HF_REPLY), Field::U32(v) if v == serial) {
                return true;
            }
            self.msg_consume();
        }
    }

    /// the signal at rbuf is the Response of our request (by its request path or handle)
    fn is_our_response(&self) -> bool {
        if self.msg_type() != MSG_SIGNAL || !self.field_is(HF_MEMBER, S_RESPONSE) {
            return false;
        }
        match self.msg_field(HF_PATH) {
            Field::Str(a, e) => {
                let p = &self.rbuf[a..e];
                p == &self.req_path[..] || p == &self.handle[..]
            }
            _ => false,
        }
    }

    /// take the events wait() queued: window events are kept for later, input events are
    /// dropped (the dialog is modal for the user)  [keep_drain]
    fn keep_drain(&mut self) {
        while let Some(e) = crate::evq_pop() {
            if matches!(e.typ, E_RESIZE | E_DPI | E_FOCUS_IN | E_FOCUS_OUT | E_CLOSE | E_PAINT) && self.keep.len() < KEEP_MAX {
                self.keep.push(e);
            }
        }
    }

    /// queue the kept window events again  [keep_restore]
    fn keep_restore(&mut self) {
        for e in self.keep.drain(..) {
            crate::evq_push(e);
        }
    }

    /// let the window breathe while waiting (plat_wait(0) + keep_drain)
    fn breathe(&mut self) {
        crate::wait(0);
        self.keep_drain();
    }

    // ------------------------------------------------------------ body reading

    fn rd_body_begin(&mut self) {
        self.rp = self.mhdr;
    }

    fn rd_align(&mut self, n: usize) {
        self.rp = ((self.rp + n - 1) & !(n - 1)).min(self.mlen);
    }

    fn rd_skip(&mut self, n: u32) {
        self.rp = (self.rp as u64 + n as u64).min(self.mlen as u64) as usize;
    }

    /// -> the value (0 past the end of the message; the reader then stays at the end)
    fn rd_u32(&mut self) -> u32 {
        self.rd_align(4);
        if self.rp + 4 > self.mlen {
            self.rp = self.mlen;
            return 0;
        }
        let v = self.rd32(self.rp);
        self.rp += 4;
        v
    }

    fn rd_f64(&mut self) -> f64 {
        self.rd_align(8);
        if self.rp + 8 > self.mlen {
            self.rp = self.mlen;
            return 0.0;
        }
        let mut b = [0u8; 8];
        b.copy_from_slice(&self.rbuf[self.rp..self.rp + 8]);
        self.rp += 8;
        f64::from_le_bytes(b)
    }

    /// a 's' / 'o' value -> its bytes (None when it does not fit into the message or is not
    /// NUL-terminated)
    fn rd_str(&mut self) -> Option<Vec<u8>> {
        let n = self.rd_u32() as u64;
        let s = self.rp as u64;
        let e = s + n + 1;
        if e > self.mlen as u64 || self.rbuf[e as usize - 1] != 0 {
            return None;
        }
        self.rp = e as usize;
        Some(self.rbuf[s as usize..e as usize - 1].to_vec())
    }

    /// a 'g' value ("" when it does not fit into the message)
    fn rd_sig(&mut self) -> Vec<u8> {
        let p = self.rp;
        if p >= self.mlen {
            self.rp = self.mlen;
            return Vec::new();
        }
        let n = self.rbuf[p] as usize;
        let e = p + 1 + n + 1;
        if e > self.mlen || self.rbuf[e - 1] != 0 {
            self.rp = self.mlen;
            return Vec::new();
        }
        self.rp = e;
        self.rbuf[p + 1..e - 1].to_vec()
    }

    /// skip one value of the type at sig[i] -> the index after the type  [skip_value]
    fn skip_value(&mut self, sig: &[u8], i: usize) -> usize {
        self.skip_depth += 1;
        let r = self.skip_value_in(sig, i);
        self.skip_depth -= 1;
        r
    }

    fn skip_value_in(&mut self, sig: &[u8], i: usize) -> usize {
        let at = |k: usize| sig.get(k).copied().unwrap_or(0);
        if self.skip_depth > SKIP_DEPTH_MAX {
            // nested deeper than any real portal reply: give up on the rest of the message
            self.rp = self.mlen;
            return sig_end(sig, i);
        }
        let c = at(i);
        self.rd_align(type_align(c));
        match c {
            b'y' => self.rd_skip(1),
            b'n' | b'q' => self.rd_skip(2),
            b'x' | b't' | b'd' => self.rd_skip(8),
            b's' | b'o' => {
                self.rd_str();
            }
            b'g' => {
                self.rd_sig();
            }
            b'v' => {
                let s = self.rd_sig();
                self.skip_value(&s, 0);
            }
            b'a' => {
                let n = self.rd_u32();
                self.rd_align(type_align(at(i + 1)));
                self.rd_skip(n);
                return sig_end(sig, i + 1);
            }
            b'(' | b'{' => {
                let mut k = i + 1;
                loop {
                    match at(k) {
                        0 => return k,
                        b')' | b'}' => return k + 1,
                        _ => k = self.skip_value(sig, k),
                    }
                }
            }
            _ => self.rd_skip(4), // b i u h
        }
        i + 1
    }

    // ------------------------------------------------------------ the portal calls

    fn set_bus_call(&mut self, member: &'static [u8], sig: Option<&'static [u8]>) {
        self.m_path = S_BUS_PATH;
        self.m_iface = S_BUS_NAME;
        self.m_member = member;
        self.m_dest = S_BUS_NAME;
        self.m_sig = sig;
        self.body_reset();
    }

    /// Hello (serial 1) -> our unique name
    fn hello(&mut self) -> Option<Vec<u8>> {
        self.set_bus_call(S_HELLO, None);
        if !self.msg_send(1) || !self.wait_reply(1, T_HELLO) {
            return None;
        }
        self.rd_body_begin();
        let name = self.rd_str()?;
        Some(name)
    }

    /// req_path = .../request/<sender>/<token>  [make_request_path]
    fn make_request_path(&mut self, uniq: &[u8]) {
        self.token_n = self.token_n.wrapping_add(1);
        self.token = copy_cap(format!("jfgui{}", self.token_n).as_bytes(), 32);
        let mut p = b"/org/freedesktop/portal/desktop/request/".to_vec();
        // the unique name comes from the bus: at most up to MP_NAME_END
        let mut s = cstr(uniq);
        if s.first() == Some(&b':') {
            s = &s[1..];
        }
        for &c in s {
            if p.len() >= MP_NAME_END {
                break;
            }
            p.push(if c == b'.' { b'_' } else { c });
        }
        p.push(b'/');
        let tok = self.token.clone();
        cat_cap(&mut p, &tok, 256);
        self.req_path = p;
        self.handle.clear();
    }

    /// AddMatch for the Response signal of our request (message serial)
    fn add_match(&mut self, serial: u32) -> bool {
        let mut t = copy_cap(S_MATCH1, 512);
        let rp = self.req_path.clone();
        cat_cap(&mut t, &rp, 512);
        cat_cap(&mut t, b"'", 512);
        self.set_bus_call(S_ADDMATCH, Some(b"s"));
        self.b_str(&t);
        self.msg_send(serial)
    }

    /// Hello + request path + AddMatch (serials 1, 2)
    fn hello_match(&mut self) -> bool {
        let Some(name) = self.hello() else { return false };
        self.make_request_path(&name);
        self.msg_consume();
        self.add_match(2)
    }

    /// AddMatch for the portal's end (a dialog waiting for its Response)
    fn owner_match(&mut self, serial: u32) -> bool {
        self.set_bus_call(S_ADDMATCH, Some(b"s"));
        self.b_str(S_MATCH_OWNER);
        self.msg_send(serial)
    }

    /// the message is NameOwnerChanged of the portal to nobody (it ended)
    fn is_portal_gone(&mut self) -> bool {
        if self.msg_type() != MSG_SIGNAL || !self.field_is(HF_MEMBER, S_NAME_OWNER_CHANGED) {
            return false;
        }
        self.rd_body_begin();
        let name = self.rd_str();
        let _old = self.rd_str();
        let new = self.rd_str();
        name.as_deref() == Some(S_PORTAL_NAME) && new.is_some_and(|n| cstr(&n).is_empty())
    }

    // ------------------------------------------------------------ FileChooser

    /// see the file header  [plat_file_dialog]
    fn file_dialog(&mut self, flags: u32, path: &mut String, exts: &[&[u8]]) -> (i32, u32) {
        self.keep.clear();
        if !self.bus_open() {
            return (-1, 0);
        }
        let r = self.file_dialog_call(flags, path, exts);
        self.bus_close();
        self.keep_restore();
        r
    }

    fn file_dialog_call(&mut self, flags: u32, path: &mut String, exts: &[&[u8]]) -> (i32, u32) {
        if !self.hello_match() || !self.owner_match(9) {
            return (-1, 0);
        }
        // FileChooser.SaveFile / OpenFile
        self.build_call(flags, path.as_bytes(), exts);
        if !self.msg_send(3) || !self.wait_reply(3, T_CALL) || self.msg_type() == MSG_ERROR {
            return (-1, 0);
        }
        self.rd_body_begin();
        if let Some(h) = self.rd_str() {
            self.handle = copy_cap(&h, 256);
        }
        self.msg_consume();
        // the user is choosing: wait for Response, keep the window alive
        let r = self.wait_response();
        if r <= 0 {
            return (r, 0);
        }
        // a file: path from the URI, extension index from the chosen filter
        let Some(p) = self.uri_to_path(PATH_CAP) else { return (0, 0) };
        let mut p = p;
        let index = if flags & FD_FOLDER != 0 { 0 } else { self.filter_index(exts) };
        if flags & (FD_SAVE | FD_FOLDER) == FD_SAVE {
            add_ext(&mut p, PATH_CAP, exts, index);
        }
        *path = String::from_utf8_lossy(&p).into_owned();
        (1, index)
    }

    /// g_m_* and the body of FileChooser.SaveFile / OpenFile  [build_call]
    fn build_call(&mut self, flags: u32, path: &[u8], exts: &[&[u8]]) {
        let folder = flags & FD_FOLDER != 0;
        let save = !folder && flags & FD_SAVE != 0;
        self.m_path = S_PORTAL_PATH;
        self.m_iface = S_FILECHOOSER;
        self.m_member = if save { b"SaveFile" } else { b"OpenFile" };
        self.m_dest = S_PORTAL_NAME;
        self.m_sig = Some(b"ssa{sv}");
        self.body_reset();
        self.b_str(&parent());
        self.b_str(if save { b"Speichern unter" } else if folder { "Ordner auswählen".as_bytes() } else { "Öffnen".as_bytes() });
        // options a{sv}
        let arr = self.b_array_begin(8);
        self.b_dict_key(b"handle_token", b"s");
        let tok = self.token.clone();
        self.b_str(&tok);
        self.b_dict_key(b"modal", b"b");
        self.b_u32(1);
        if folder {
            // FD_FOLDER: OpenFile with "directory" (portal version 3), starting in path itself
            self.b_dict_key(b"directory", b"b");
            self.b_u32(1);
            if !path.is_empty() {
                self.b_dict_key(b"current_folder", b"ay");
                let a2 = self.b_array_begin(1);
                let mut f = path.to_vec();
                f.push(0);
                self.b_bytes(&f);
                self.b_array_end(a2);
            }
            self.b_array_end(arr);
            return;
        }
        // suggested folder (both) and name (save)
        self.split_path(path);
        if save && !self.name.is_empty() {
            self.b_dict_key(b"current_name", b"s");
            let n = self.name.clone();
            self.b_str(&n);
        }
        if !self.folder.is_empty() {
            self.b_dict_key(b"current_folder", b"ay");
            let a2 = self.b_array_begin(1);
            let mut f = self.folder.clone();
            f.push(0); // with the NUL
            self.b_bytes(&f);
            self.b_array_end(a2);
        }
        if !exts.is_empty() {
            // "filters": a(sa(us))
            self.b_dict_key(b"filters", b"a(sa(us))");
            let a2 = self.b_array_begin(8);
            if !save {
                self.b_filter(-1, exts); // open: all supported files first
            }
            for i in 0..exts.len() {
                self.b_filter(i as i32, exts);
            }
            self.b_array_end(a2);
            // "current_filter": (sa(us))
            self.b_dict_key(b"current_filter", b"(sa(us))");
            let mut cur = -1;
            if save {
                cur = ((flags >> FD_EXT_SHIFT) & 0xFF) as i32;
                if cur as usize >= exts.len() {
                    cur = 0;
                }
            }
            self.b_filter(cur, exts);
        }
        self.b_array_end(arr);
    }

    /// one (sa(us)) filter: name and glob patterns (i = -1: all entries); every extension of
    /// an entry ("a;b=...") becomes a pattern  [b_filter]
    fn b_filter(&mut self, i: i32, exts: &[&[u8]]) {
        let fname = filter_name(i, exts);
        self.b_align(8);
        self.b_str(&fname);
        let arr = self.b_array_begin(8);
        let mut j = if i == -1 { 0 } else { i as usize };
        while let Some(&e) = exts.get(j) {
            let mut p = 0;
            loop {
                let ext = ext_copy(&e[p..]);
                self.b_align(8);
                self.b_u32(0); // 0 = glob pattern
                // GTK's file filters (GNOME's portal) match globs case-sensitively: every
                // letter both ways ("*.[jJ][pP][gG]"), so IMG_0001.JPG shows too
                let mut pat = copy_cap(b"*.", 128);
                for &c in cstr(&ext) {
                    if pat.len() + 4 >= 512 {
                        break;
                    }
                    if c.is_ascii_alphabetic() {
                        pat.extend_from_slice(&[b'[', c.to_ascii_lowercase(), c.to_ascii_uppercase(), b']']);
                    } else {
                        pat.push(c);
                    }
                }
                self.b_str(&pat);
                // the next extension of this entry ("a;b=...")
                let mut k = p;
                let mut next = None;
                while k < e.len() && e[k] != 0 && e[k] != b'=' {
                    k += 1;
                    if e[k - 1] == b';' {
                        next = Some(k);
                        break;
                    }
                }
                match next {
                    Some(n) => p = n,
                    None => break,
                }
            }
            if i != -1 {
                break;
            }
            j += 1;
        }
        self.b_array_end(arr);
    }

    /// folder = the existing folder part (absolute), name = the name part  [split_path]
    fn split_path(&mut self, path: &[u8]) {
        self.folder.clear();
        self.name.clear();
        let tmp = copy_cap(path, 512);
        match tmp.iter().rposition(|&c| c == b'/') {
            None => self.name = copy_cap(&tmp, 256),
            Some(k) => {
                self.name = copy_cap(&tmp[k + 1..], 256);
                let dir = &tmp[..k];
                if dir.first() == Some(&b'/') && crate::file_exists_raw(&String::from_utf8_lossy(dir)) == 2 {
                    self.folder = copy_cap(dir, 512);
                }
            }
        }
    }

    /// "file:///a%20b" -> "/a b" (at most cap - 1 bytes)  [uri_to_path]
    fn uri_to_path(&self, cap: usize) -> Option<Vec<u8>> {
        let u = &self.uri;
        if !u.starts_with(b"file://") {
            return None;
        }
        let at = |k: usize| u.get(k).copied().unwrap_or(0);
        let mut k = 7;
        if at(k) != b'/' {
            // "file://host/path": drop the host
            loop {
                if k >= u.len() || at(k) == 0 {
                    return None;
                }
                if at(k) == b'/' {
                    break;
                }
                k += 1;
            }
        }
        let mut out = Vec::new();
        while out.len() < cap - 1 && k < u.len() && at(k) != 0 {
            let mut c = at(k);
            k += 1;
            if c == b'%' {
                // "%XY": both digits inside the URI, neither the NUL
                if k + 2 > URI_CAP || at(k) == 0 || at(k + 1) == 0 {
                    break;
                }
                c = (hexval(at(k)) << 4 | hexval(at(k + 1))) as u8;
                k += 2;
                if c == 0 {
                    break; // "%00": the C string of the assembly ends here
                }
            }
            out.push(c);
        }
        Some(out)
    }

    /// the index of the entry whose description is the chosen filter (0 if none)  [filter_index]
    fn filter_index(&self, exts: &[&[u8]]) -> u32 {
        if self.filter.is_empty() {
            return 0;
        }
        for i in 0..exts.len() {
            if filter_name(i as i32, exts) == self.filter {
                return i as u32;
            }
        }
        0
    }

    /// 1 file chosen (uri, filter), 0 cancelled, -1 lost connection; services the display
    /// meanwhile  [wait_response]
    fn wait_response(&mut self) -> i32 {
        loop {
            if self.msg_ready() {
                if self.is_our_response() {
                    let r = self.parse_response();
                    self.msg_consume();
                    return r;
                }
                if self.is_portal_gone() {
                    self.msg_consume();
                    return -1; // no answer will come
                }
                self.msg_consume();
                continue;
            }
            let r = self.bus_fill(40);
            if r < 0 {
                return -1;
            }
            if r > 0 {
                continue;
            }
            self.breathe(); // nothing from the bus: let the window breathe
        }
    }

    /// body "ua{sv}": code, results ("uris" as -> uri, "current_filter" (sa(us)) -> filter)
    /// -> 1 file chosen, 0 cancelled  [parse_response]
    fn parse_response(&mut self) -> i32 {
        self.uri.clear();
        self.filter.clear();
        self.rd_body_begin();
        if self.rd_u32() != 0 {
            return 0;
        }
        let len = self.rd_u32();
        self.rd_align(8);
        let end = (len as usize + self.rp) & 0xFFFF_FFFF;
        loop {
            self.rd_align(8);
            if self.rp >= end {
                break;
            }
            let Some(key) = self.rd_str() else { break };
            let sig = self.rd_sig();
            if key == b"uris" {
                if sig == b"as" {
                    let n = self.rd_u32();
                    let start = self.rp;
                    if n != 0 {
                        if let Some(s) = self.rd_str() {
                            self.uri = copy_cap(&s, URI_CAP); // the first URI
                        }
                    }
                    self.rp = start; // back to the array start, then skip it
                    self.rd_skip(n);
                    continue;
                }
            } else if key == b"current_filter" && sig == b"(sa(us))" {
                self.rd_align(8);
                let start = self.rp;
                if let Some(s) = self.rd_str() {
                    self.filter = copy_cap(&s, 256); // the filter name
                }
                self.rp = start;
            }
            self.skip_value(&sig, 0);
        }
        (!self.uri.is_empty()) as i32
    }

    // ------------------------------------------------------------ OpenURI

    /// close the connection a previous open_folder kept open  [of_release]
    fn of_release(&mut self) {
        if self.of_keep > 0 {
            sys_close(self.of_keep);
            self.of_keep = 0;
        }
    }

    /// see the file header  [plat_open_folder]
    fn open_folder(&mut self, path: &[u8]) -> i32 {
        self.of_release();
        let path = cstr(path);
        if path.is_empty() {
            return -1;
        }
        // absolute path
        let mut of = Vec::new();
        if path[0] != b'/' {
            of = copy_cap(crate::cwd().as_bytes(), OF_CAP);
            cat_cap(&mut of, b"/", OF_CAP);
        }
        cat_cap(&mut of, path, OF_CAP);
        // an O_PATH descriptor (the path must exist)
        let mut z = of.clone();
        z.push(0);
        // SAFETY: openat with a NUL-terminated path.
        let fd = unsafe { syscall6(SYS_OPENAT, AT_FDCWD as usize, z.as_ptr() as usize, O_PATH | O_CLOEXEC, 0, 0, 0) };
        if fd < 0 {
            return -1;
        }
        let fd = fd as i32;
        let mut r = -1;
        if of_uri(&mut self.uri, &of) {
            if env_is_1("jfgui_OPEN_DRYRUN") {
                let line = [
                    &b"plat_open_folder: OpenURI.OpenDirectory(fd of "[..],
                    &of,
                    b"), fallback OpenURI.OpenURI(",
                    &self.uri,
                    b")\n",
                ]
                .concat();
                crate::write_stdout(&line);
                r = 0;
            } else {
                r = self.of_send(fd);
            }
        }
        sys_close(fd);
        r
    }

    /// see the file header  [plat_open_url]
    fn open_url(&mut self, url: &[u8]) -> i32 {
        self.of_release();
        let url = cstr(url);
        if url.is_empty() || url.len() - 1 > URI_CAP - 2 {
            return -1; // empty or too long
        }
        self.uri = copy_cap(url, URI_CAP);
        if env_is_1("jfgui_OPEN_DRYRUN") {
            let line = [&b"plat_open_url: OpenURI.OpenURI("[..], &self.uri, b")\n"].concat();
            crate::write_stdout(&line);
            return 0;
        }
        self.of_send(-1)
    }

    /// the portal call of open_folder / open_url: fd >= 0 OpenDirectory with that descriptor
    /// first, else (or when rejected) OpenURI with uri  [of_send]
    fn of_send(&mut self, fd: i32) -> i32 {
        self.keep.clear();
        self.neg_fd = fd >= 0; // descriptors only when there is one
        let ok = self.bus_open();
        self.neg_fd = false;
        if !ok {
            return -1;
        }
        let r = self.of_send_call(fd);
        self.bus_close();
        self.keep_restore();
        r
    }

    fn of_send_call(&mut self, fd: i32) -> i32 {
        if !self.hello_match() {
            return -1;
        }
        let mut done = false;
        if fd >= 0 && self.fd_ok {
            // OpenDirectory with the descriptor
            self.of_build(true);
            self.m_nfds = 1;
            self.m_sendfd = fd;
            let ok = self.msg_send(3);
            self.m_nfds = 0;
            if !ok {
                return -1;
            }
            done = self.of_answer(3);
        }
        if !done {
            // rejected (or no descriptor): OpenURI with a file:// URI
            self.of_build(false);
            if !self.msg_send(4) || !self.of_answer(4) {
                return -1;
            }
        }
        // the Response (or keep the request alive)
        match self.of_wait(OF_WAIT) {
            -2 => {
                self.of_keep = self.fd;
                self.fd = -1;
                0
            }
            0 => 0,
            _ => -1,
        }
    }

    /// the call returned (request handle stored) -> true; false on an error reply / no answer
    /// [of_answer]
    fn of_answer(&mut self, serial: u32) -> bool {
        if !self.wait_reply(serial, T_CALL) {
            return false;
        }
        if self.msg_type() == MSG_ERROR {
            self.msg_consume();
            return false;
        }
        self.handle.clear();
        self.rd_body_begin();
        if let Some(h) = self.rd_str() {
            self.handle = copy_cap(&h, 256);
        }
        self.msg_consume();
        true
    }

    /// g_m_* and the body of OpenURI.OpenDirectory(s parent, h 0, a{sv}) or
    /// OpenURI.OpenURI(s parent, s uri, a{sv}); options: handle_token  [of_build]
    fn of_build(&mut self, with_fd: bool) {
        self.m_path = S_PORTAL_PATH;
        self.m_iface = S_OPENURI;
        self.m_member = if with_fd { b"OpenDirectory" } else { b"OpenURI" };
        self.m_dest = S_PORTAL_NAME;
        self.m_sig = Some(if with_fd { b"sha{sv}" } else { b"ssa{sv}" });
        self.body_reset();
        self.b_str(&parent());
        if with_fd {
            self.b_u32(0); // 'h': index 0 of the message's descriptors
        } else {
            let u = self.uri.clone();
            self.b_str(&u);
        }
        let arr = self.b_array_begin(8);
        self.b_dict_key(b"handle_token", b"s");
        let t = self.token.clone();
        self.b_str(&t);
        self.b_array_end(arr);
    }

    /// the Response code of our request (0 ok, 1 cancelled, 2 failed), -2 no answer within
    /// ms, -1 connection lost; services the window meanwhile.  parse: a code 0 Response's
    /// results go to page (Print)  [of_wait, prn_wait]
    fn resp_wait(&mut self, ms: u32, mut page: Option<&mut PPage>) -> i32 {
        let deadline = crate::time_us() + ms as u64 * 1000;
        loop {
            if self.msg_ready() {
                if self.is_our_response() {
                    self.rd_body_begin();
                    let code = self.rd_u32();
                    if code == 0 {
                        if let Some(p) = page.as_deref_mut() {
                            self.prn_results(p);
                        }
                    }
                    self.msg_consume();
                    return code as i32;
                }
                self.msg_consume();
                continue;
            }
            if crate::time_us() >= deadline {
                return -2;
            }
            let r = self.bus_fill(20);
            if r < 0 {
                return -1;
            }
            if r > 0 {
                continue;
            }
            if crate::disp_backend() != 0 {
                self.breathe(); // a window to keep alive
            }
        }
    }

    fn of_wait(&mut self, ms: u32) -> i32 {
        self.resp_wait(ms, None)
    }

    // ------------------------------------------------------------ Print

    /// PreparePrint with the job's paper / orientation as suggestion; page gets the dialog's
    /// page setup and the token -> 1 ok, 0 cancelled, -1 no bus / no portal / no descriptor
    /// passing.  The connection stays open for prn_send.  [portal_print_prepare]
    fn prn_prepare(&mut self, job: &PrintJob, page: &mut PPage, title: &[u8]) -> i32 {
        self.prn_end();
        self.keep.clear();
        self.neg_fd = true;
        let ok = self.bus_open();
        self.neg_fd = false;
        self.prn_open = true;
        if !ok || !self.fd_ok {
            return -1; // Print needs descriptor passing
        }
        let Some(u) = self.hello() else { return -1 };
        self.prn_uniq = copy_cap(&u, 128);
        self.msg_consume();
        if !self.prn_match(2) {
            return -1;
        }
        self.prn_build_prepare(job, title);
        if !self.msg_send(3) || !self.of_answer(3) {
            return -1;
        }
        match self.resp_wait(PRN_T_DIALOG, Some(page)) {
            r if r < 0 => -1,
            0 => 1,
            _ => 0, // 1 cancelled, 2 ended otherwise
        }
    }

    /// Print(parent, title, h fd, {handle_token, modal, token}) -> 0 printed, 1 cancelled,
    /// -1 failed  [portal_print_send]
    fn prn_send(&mut self, title: &[u8], fd: i32, token: u32) -> i32 {
        if !self.prn_open || self.fd < 0 || !self.prn_match(4) {
            return -1;
        }
        self.prn_build_print(title, token);
        self.m_nfds = 1;
        self.m_sendfd = fd;
        let ok = self.msg_send(5);
        self.m_nfds = 0;
        if !ok || !self.of_answer(5) {
            return -1;
        }
        match self.resp_wait(PRN_T_PRINT, None) {
            r @ (0 | 1) => r,
            _ => -1, // 2 failed, < 0 lost / no answer
        }
    }

    /// close the connection of prn_prepare  [portal_print_end]
    fn prn_end(&mut self) {
        if self.prn_open {
            self.prn_open = false;
            self.bus_close();
            self.keep_restore();
        }
    }

    /// a new request path (token) and AddMatch on its Response  [prn_match]
    fn prn_match(&mut self, serial: u32) -> bool {
        let u = self.prn_uniq.clone();
        self.make_request_path(&u);
        self.add_match(serial)
    }

    /// g_m_* for a Print portal call, body = the parent window string  [prn_head]
    fn prn_head(&mut self, member: &'static [u8], sig: &'static [u8]) {
        self.m_member = member;
        self.m_sig = Some(sig);
        self.m_path = S_PORTAL_PATH;
        self.m_iface = S_PRN_IFACE;
        self.m_dest = S_PORTAL_NAME;
        self.body_reset();
        self.b_str(&parent());
    }

    fn prn_dict_s(&mut self, key: &[u8], s: &[u8]) {
        self.b_dict_key(key, b"s");
        self.b_str(s);
    }

    fn prn_dict_d(&mut self, key: &[u8], v: f64) {
        self.b_dict_key(key, b"d");
        self.b_f64(v);
    }

    fn prn_dict_u(&mut self, key: &[u8], sig: &[u8], v: u32) {
        self.b_dict_key(key, sig);
        self.b_u32(v);
    }

    /// PreparePrint(s parent, s title, a{sv} settings, a{sv} page_setup, a{sv} options)
    /// [prn_build_prepare]
    fn prn_build_prepare(&mut self, job: &PrintJob, title: &[u8]) {
        self.prn_head(b"PreparePrint", b"ssa{sv}a{sv}a{sv}");
        self.b_str(title);
        let p = PRN_PAPERS[if job.paper >= PAPER_N { 0 } else { job.paper as usize }];
        let orient: &[u8] = if job.flags & PJF_LANDSCAPE != 0 { b"landscape" } else { b"portrait" };
        // settings
        let a = self.b_array_begin(8);
        self.prn_dict_s(b"orientation", orient);
        self.b_array_end(a);
        // page setup
        let a = self.b_array_begin(8);
        self.prn_dict_s(b"Name", p.2);
        self.prn_dict_s(b"DisplayName", p.3);
        self.prn_dict_d(b"Width", p.0);
        self.prn_dict_d(b"Height", p.1);
        self.prn_dict_s(b"Orientation", orient);
        self.b_array_end(a);
        // options
        let a = self.b_array_begin(8);
        let t = self.token.clone();
        self.prn_dict_s(b"handle_token", &t);
        self.prn_dict_u(b"modal", b"b", 1);
        self.b_array_end(a);
    }

    /// Print(s parent, s title, h 0, a{sv} options)  [prn_build_print]
    fn prn_build_print(&mut self, title: &[u8], token: u32) {
        self.prn_head(b"Print", b"ssha{sv}");
        self.b_str(title);
        self.b_u32(0); // 'h': index 0 of the message's descriptors
        let a = self.b_array_begin(8);
        let t = self.token.clone();
        self.prn_dict_s(b"handle_token", &t);
        self.prn_dict_u(b"modal", b"b", 1);
        self.prn_dict_u(b"token", b"u", token);
        self.b_array_end(a);
    }

    /// the end offset of the a{sv} array starting at the reader (never past the message)
    /// [prn_dict_end]
    fn prn_dict_end(&mut self) -> usize {
        let n = self.rd_u32() as usize;
        self.rd_align(8);
        let room = self.mlen.saturating_sub(self.rp);
        self.rp + n.min(room)
    }

    /// the results a{sv} of PreparePrint: "token" u, "page-setup" a{sv}  [prn_results]
    fn prn_results(&mut self, page: &mut PPage) {
        let end = self.prn_dict_end();
        loop {
            self.rd_align(8);
            if self.rp >= end {
                return;
            }
            let Some(key) = self.rd_str() else { return };
            let sig = self.rd_sig();
            if key == b"token" {
                if sig == b"u" {
                    page.token = self.rd_u32();
                    continue;
                }
            } else if key == b"page-setup" && sig == b"a{sv}" {
                self.prn_setup(page);
                continue;
            }
            self.skip_value(&sig, 0);
        }
    }

    /// page-setup a{sv}: Width / Height / Margin* (d, mm), Orientation (s) -> page (pt)
    /// [prn_setup]
    fn prn_setup(&mut self, page: &mut PPage) {
        let mut mm = [0f64; 6];
        let mut land = 0u32;
        let end = self.prn_dict_end();
        loop {
            self.rd_align(8);
            if self.rp >= end {
                break;
            }
            let Some(key) = self.rd_str() else { break };
            let sig = self.rd_sig();
            if sig.first() == Some(&b'd') {
                if sig.len() == 1 {
                    let v = self.rd_f64();
                    if let Some(k) = PRN_DKEYS.iter().position(|&n| n == &key[..]) {
                        mm[k] = v;
                    }
                    continue;
                }
            } else if key == b"Orientation" && sig == b"s" {
                if let Some(s) = self.rd_str() {
                    land = s.windows(9).any(|w| w == b"landscape") as u32;
                }
                continue;
            }
            self.skip_value(&sig, 0);
        }
        let (mut w, mut h) = (mm[0], mm[1]);
        if !(w > 0.0) || !(h > 0.0) {
            return;
        }
        w *= PRN_MM_PT;
        h *= PRN_MM_PT;
        page.flags = land;
        if land != 0 {
            core::mem::swap(&mut w, &mut h);
        }
        page.w = w;
        page.h = h;
        page.t = mm[2] * PRN_MM_PT;
        page.b = mm[3] * PRN_MM_PT;
        page.l = mm[4] * PRN_MM_PT;
        page.r = mm[5] * PRN_MM_PT;
    }
}

// ---------------------------------------------------------------- signatures

/// the index after one complete type at sig[i]  [sig_end]
fn sig_end(sig: &[u8], mut i: usize) -> usize {
    let at = |k: usize| sig.get(k).copied().unwrap_or(0);
    while at(i) == b'a' {
        i += 1;
    }
    if at(i) != b'(' && at(i) != b'{' {
        return i + 1;
    }
    let mut depth = 0i32;
    loop {
        let c = at(i);
        if c == 0 {
            return i;
        }
        i += 1;
        match c {
            b'(' | b'{' => depth += 1,
            b')' | b'}' => {
                depth -= 1;
                if depth == 0 {
                    return i;
                }
            }
            _ => {}
        }
    }
}

/// the alignment of a type code  [type_align]
fn type_align(c: u8) -> usize {
    match c {
        b'n' | b'q' => 2,
        b'x' | b't' | b'd' | b'(' | b'{' => 8,
        b'b' | b'i' | b'u' | b'h' | b's' | b'o' | b'a' => 4,
        _ => 1,
    }
}

/// 0..15 (0 for anything else)  [hexval]
fn hexval(c: u8) -> u32 {
    let c = c as u32;
    let d = c.wrapping_sub(48);
    if d <= 9 {
        return d;
    }
    let x = (c | 32).wrapping_sub(87);
    if x <= 15 { x } else { 0 }
}

/// the first extension of "a;b=description" (at most 30 bytes)  [ext_copy]
fn ext_copy(e: &[u8]) -> Vec<u8> {
    let mut v = Vec::new();
    for &c in e {
        if c == 0 || c == b'=' || c == b';' || v.len() >= 30 {
            break;
        }
        v.push(c);
    }
    v
}

/// the description of entry i ("PNG-Datei" without one); i = -1: "Alle unterstützten
/// Dateien"  [filter_name]
fn filter_name(i: i32, exts: &[&[u8]]) -> Vec<u8> {
    if i == -1 {
        return copy_cap(S_ALL_FILES, 128);
    }
    let e = exts.get(i as usize).copied().unwrap_or(b"");
    match e.iter().position(|&c| c == b'=') {
        Some(k) => copy_cap(&e[k + 1..], 128),
        None => {
            let mut x = ext_copy(e);
            x.make_ascii_uppercase();
            let mut f = copy_cap(&x, 128);
            cat_cap(&mut f, b"-Datei", 128);
            f
        }
    }
}

/// append ".ext" of entry index when the name has none  [add_ext]
fn add_ext(path: &mut Vec<u8>, cap: usize, exts: &[&[u8]], index: u32) {
    let from = path.iter().rposition(|&c| c == b'/').unwrap_or(0);
    if path[from..].contains(&b'.') {
        return;
    }
    let Some(e) = exts.get(index as usize) else { return };
    let ext = ext_copy(e);
    cat_cap(path, b".", cap);
    cat_cap(path, &ext, cap);
}

/// uri = "file://" + path percent-encoded (A-Z a-z 0-9 - . _ ~ / unchanged) -> false when too
/// long  [of_uri]
fn of_uri(uri: &mut Vec<u8>, path: &[u8]) -> bool {
    uri.clear();
    uri.extend_from_slice(b"file://");
    for &c in path {
        if uri.len() >= URI_CAP - 4 {
            return false;
        }
        if c.is_ascii_alphanumeric() || matches!(c, b'-' | b'.' | b'_' | b'~' | b'/') {
            uri.push(c);
        } else {
            uri.push(b'%');
            uri.push(S_HEXU[(c >> 4) as usize]);
            uri.push(S_HEXU[(c & 15) as usize]);
        }
    }
    uri.len() < URI_CAP - 4 // (the room is checked before the terminator too)
}

// ---------------------------------------------------------------- the API

/// the portal's FileChooser (see the file header)  [plat_file_dialog]
pub fn file_dialog(flags: u32, path: &mut String, exts: &[&str]) -> (i32, u32) {
    let list: Vec<&[u8]> = exts.iter().map(|e| cstr(e.as_bytes())).take_while(|e| !e.is_empty()).collect();
    with_bus(|b| b.file_dialog(flags, path, &list))
}

/// show a folder / a file in the file manager (OpenURI.OpenDirectory) -> 0 / -1  [plat_open_folder]
pub fn open_folder(path: &str) -> i32 {
    if !crate::policy::fs_allowed(path, crate::policy::FS_READ) {
        return -1;
    }
    with_bus(|b| b.open_folder(path.as_bytes()))
}

/// a web page in the default application (OpenURI.OpenURI) -> 0 / -1  [plat_open_url]
pub fn open_url(url: &str) -> i32 {
    if !crate::policy::url_allowed(url) {
        return -1;
    }
    with_bus(|b| b.open_url(url.as_bytes()))
}

/// (crate, linux_print.rs) PreparePrint -> 1 ok, 0 cancelled, -1 unsupported  [portal_print_prepare]
pub(crate) fn portal_print_prepare(job: &PrintJob, page: &mut PPage, title: &str) -> i32 {
    with_bus(|b| b.prn_prepare(job, page, title.as_bytes()))
}

/// (crate) Print with the PDF in fd -> 0 printed, 1 cancelled, -1 failed  [portal_print_send]
pub(crate) fn portal_print_send(title: &str, fd: i32, token: u32) -> i32 {
    with_bus(|b| b.prn_send(title.as_bytes(), fd, token))
}

/// (crate) close the connection of portal_print_prepare  [portal_print_end]
pub(crate) fn portal_print_end() {
    with_bus(|b| b.prn_end())
}
