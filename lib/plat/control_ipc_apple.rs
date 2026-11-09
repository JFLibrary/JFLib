// Nonblocking Unix sockets restricted to the effective user on macOS and iOS (libSystem; the same
// promises as control_ipc_linux.rs).  The peer's user comes from getpeereid.  Darwin has no abstract
// socket names: the app's single instance is a socket file in a private folder of the user, a stale
// one (nobody answers) is replaced.  Socket paths never accept external directory or file input.

use alloc::{format, string::String};
use core::ffi::{c_char, c_int, c_void};

use crate::apple::ffi;

const AF_UNIX: c_int = 1;
const SOCK_STREAM: c_int = 1;
const SOL_SOCKET: c_int = 0xffff;
const SO_NOSIGPIPE: c_int = 0x1022;
const F_SETFD: c_int = 2;
const F_SETFL: c_int = 4;
const EAGAIN: i32 = 35;
const EADDRINUSE: i32 = 48;
const ECONNREFUSED: i32 = 61;
const SUN_PATH: usize = 104;

#[link(name = "System")]
unsafe extern "C" {
    fn socket(domain: c_int, ty: c_int, proto: c_int) -> c_int;
    fn bind(fd: c_int, a: *const u8, len: u32) -> c_int;
    fn listen(fd: c_int, n: c_int) -> c_int;
    fn accept(fd: c_int, a: *mut u8, len: *mut u32) -> c_int;
    fn connect(fd: c_int, a: *const u8, len: u32) -> c_int;
    fn send(fd: c_int, b: *const c_void, n: usize, flags: c_int) -> isize;
    fn setsockopt(fd: c_int, level: c_int, name: c_int, v: *const c_void, len: u32) -> c_int;
    fn getpeereid(fd: c_int, uid: *mut u32, gid: *mut u32) -> c_int;
    fn geteuid() -> u32;
    fn chmod(path: *const c_char, mode: u16) -> c_int;
}

fn euid() -> u32 {
    // SAFETY: no arguments.
    unsafe { geteuid() }
}

/// a Unix stream socket: close-on-exec, no SIGPIPE, nonblocking when asked
fn new_socket(nonblocking: bool) -> Result<i32, i32> {
    // SAFETY: plain socket creation; fcntl / setsockopt on the own descriptor.
    unsafe {
        let fd = socket(AF_UNIX, SOCK_STREAM, 0);
        if fd < 0 {
            return Err(ffi::errno());
        }
        setup(fd, nonblocking);
        Ok(fd)
    }
}

fn setup(fd: i32, nonblocking: bool) {
    let one: c_int = 1;
    // SAFETY: fcntl / setsockopt on an owned descriptor.
    unsafe {
        ffi::fcntl(fd, F_SETFD, 1 /* FD_CLOEXEC */);
        setsockopt(fd, SOL_SOCKET, SO_NOSIGPIPE, (&one as *const c_int).cast(), 4);
        if nonblocking {
            ffi::fcntl(fd, F_SETFL, ffi::O_NONBLOCK);
        }
    }
}

/// sockaddr_un of path: sun_len, sun_family, the NUL-terminated path -> (record, length)
fn unix_addr(path: &str) -> Result<([u8; 2 + SUN_PATH], u32), i32> {
    if path.len() >= SUN_PATH {
        return Err(63); // ENAMETOOLONG
    }
    let mut a = [0u8; 2 + SUN_PATH];
    a[0] = (2 + path.len() + 1) as u8;
    a[1] = AF_UNIX as u8;
    a[2..2 + path.len()].copy_from_slice(path.as_bytes());
    Ok((a, (2 + path.len() + 1) as u32))
}

/// the uid of the process at the other end of fd
fn peer_uid(fd: i32) -> Option<u32> {
    let (mut uid, mut gid) = (u32::MAX, u32::MAX);
    // SAFETY: two writable u32 for a connected Unix socket.
    (unsafe { getpeereid(fd, &mut uid, &mut gid) } == 0).then_some(uid)
}

/// a folder only this user may enter, created when missing: /tmp/<prefix>-<uid> on macOS (short
/// enough for sun_path), $TMPDIR/<prefix> on iOS (the app's own)
fn private_dir(prefix: &str) -> Result<String, i32> {
    let uid = euid();
    let directory = if cfg!(target_os = "ios") {
        let t = crate::temp_dir().ok_or(2)?;
        format!("{t}/{prefix}")
    } else {
        format!("/tmp/{prefix}-{uid}")
    };
    let dir_z = format!("{directory}\0");
    // SAFETY: mkdir / open receive a NUL-terminated path; O_NOFOLLOW refuses a planted link.
    let fd = unsafe {
        if ffi::mkdir(dir_z.as_ptr().cast(), 0o700) < 0 && ffi::errno() != ffi::EEXIST {
            return Err(ffi::errno());
        }
        ffi::open(dir_z.as_ptr().cast(), ffi::O_RDONLY | ffi::O_DIRECTORY | ffi::O_CLOEXEC | ffi::O_NOFOLLOW, 0u32)
    };
    if fd < 0 {
        return Err(ffi::errno());
    }
    let mut st = ffi::Stat::default();
    // SAFETY: fstat writes the stat record of the own descriptor.
    let status = unsafe { ffi::fstat(fd, &mut st) };
    close(fd);
    if status < 0 {
        return Err(ffi::errno());
    }
    if st.st_uid != uid || st.st_mode as u32 & 0o170777 != 0o040700 {
        return Err(13);
    }
    Ok(directory)
}

/// Private local listener. Dropping it closes and removes only its own socket path.
pub struct Listener {
    fd: i32,
    path: String,
}

/// One nonblocking authenticated local connection.
pub struct Stream {
    fd: i32,
}

impl Listener {
    /// Bind below a verified, non-symlink, mode-0700 directory owned by the effective UID.
    pub fn bind(app: &str) -> Result<Self, i32> {
        if app.is_empty() || app.len() > 32 || !app.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-')) {
            return Err(22);
        }
        let directory = private_dir("jfgui-agent")?;
        // SAFETY: no arguments.
        let pid = unsafe { ffi::getpid() };
        let path = format!("{directory}/{app}-{pid}-{}.sock", crate::time_us());
        let (address, length) = unix_addr(&path)?;
        let fd = new_socket(true)?;
        // SAFETY: the address record and its length are valid for the call.
        if unsafe { bind(fd, address.as_ptr(), length) } < 0 {
            let e = ffi::errno();
            close(fd);
            return Err(e);
        }
        let path_z = format!("{path}\0");
        // SAFETY: the socket lives inside the checked private directory; chmod / listen on own path and fd.
        let status = unsafe { if chmod(path_z.as_ptr().cast(), 0o600) < 0 { -1 } else { listen(fd, 8) } };
        if status < 0 {
            let e = ffi::errno();
            close(fd);
            unlink(&path);
            return Err(e);
        }
        Ok(Self { fd, path })
    }
    /// Address to advertise only while control is active.
    pub fn endpoint(&self) -> &str {
        &self.path
    }
    /// Accept at most one available connection and validate its effective UID (getpeereid).
    pub fn accept(&mut self) -> Result<Option<Stream>, i32> {
        // SAFETY: a nonblocking accept without the peer's address.
        let fd = unsafe { accept(self.fd, core::ptr::null_mut(), core::ptr::null_mut()) };
        if fd < 0 {
            let e = ffi::errno();
            return if e == EAGAIN || e == ffi::EINTR { Ok(None) } else { Err(e) };
        }
        setup(fd, true);
        if peer_uid(fd) != Some(euid()) {
            close(fd);
            return Err(13);
        }
        Ok(Some(Stream { fd }))
    }
}

impl Stream {
    /// Read available bytes. None means would block; Some(0) means peer EOF.
    pub fn read(&mut self, bytes: &mut [u8]) -> Result<Option<usize>, i32> {
        // SAFETY: read receives the exclusive writable range of the caller's slice.
        result(unsafe { ffi::read(self.fd, bytes.as_mut_ptr().cast(), bytes.len()) })
    }
    /// Write available bytes without SIGPIPE (SO_NOSIGPIPE). None means would block.
    pub fn write(&mut self, bytes: &[u8]) -> Result<Option<usize>, i32> {
        // SAFETY: send reads a valid slice on a connected socket.
        result(unsafe { send(self.fd, bytes.as_ptr().cast(), bytes.len(), 0) })
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        close(self.fd);
        unlink(&self.path);
    }
}
impl Drop for Stream {
    fn drop(&mut self) {
        close(self.fd);
    }
}

fn result(value: isize) -> Result<Option<usize>, i32> {
    if value >= 0 {
        return Ok(Some(value as usize));
    }
    let e = ffi::errno();
    if e == EAGAIN || e == ffi::EINTR { Ok(None) } else { Err(e) }
}
fn close(fd: i32) {
    // SAFETY: this module calls close once for each owned descriptor.
    unsafe { ffi::close(fd) };
}
fn unlink(path: &str) {
    let path = format!("{path}\0");
    // SAFETY: a path this module created; no external path components.
    unsafe { ffi::unlink(path.as_ptr().cast()) };
}

// ---------------------------------------------------------------- the app's single instance
// (instance.rs): the socket file <private folder>/<app>.sock; the process that binds it first is the
// app's running instance.  A file nobody answers on is left over from an ended process: removed and
// bound again.  Both sides check the other's uid with getpeereid.

/// the listening side of the instance socket (blocking accept)
pub(crate) struct InstServer {
    fd: i32,
    path: String,
}

/// one exchange over the instance socket (nonblocking reads and writes)
pub(crate) struct InstConn {
    fd: i32,
}

fn inst_path(app: &str) -> Result<String, i32> {
    if app.is_empty() || app.len() > 64 || !app.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.')) {
        return Err(22);
    }
    Ok(format!("{}/{app}.sock", private_dir("jfgui-instance")?))
}

/// bind the instance socket -> None when another process already has it
pub(crate) fn inst_listen(app: &str) -> Result<Option<InstServer>, i32> {
    let path = inst_path(app)?;
    let (address, length) = unix_addr(&path)?;
    for _ in 0..2 {
        let fd = new_socket(false)?;
        // SAFETY: the address record and its length are valid for the call.
        if unsafe { bind(fd, address.as_ptr(), length) } == 0 {
            // SAFETY: listen on the bound socket.
            if unsafe { listen(fd, 4) } < 0 {
                let e = ffi::errno();
                close(fd);
                unlink(&path);
                return Err(e);
            }
            return Ok(Some(InstServer { fd, path }));
        }
        let e = ffi::errno();
        close(fd);
        if e != EADDRINUSE {
            return Err(e);
        }
        match inst_connect(app) {
            Ok(_) => return Ok(None),            // the instance runs
            Err(ECONNREFUSED) => unlink(&path), // left over: replace it
            Err(e) => return Err(e),
        }
    }
    Ok(None)
}

impl InstServer {
    /// wait for the next starting process of the same user (blocks)
    pub(crate) fn accept(&self) -> Result<InstConn, i32> {
        loop {
            // SAFETY: a blocking accept without the peer's address.
            let fd = unsafe { accept(self.fd, core::ptr::null_mut(), core::ptr::null_mut()) };
            if fd < 0 {
                let e = ffi::errno();
                if e == ffi::EINTR {
                    continue;
                }
                return Err(e);
            }
            setup(fd, true);
            if peer_uid(fd) == Some(euid()) {
                return Ok(InstConn { fd });
            }
            close(fd);
        }
    }
}

impl Drop for InstServer {
    fn drop(&mut self) {
        close(self.fd);
        unlink(&self.path);
    }
}

/// connect to the running instance (it must run as the same user)
pub(crate) fn inst_connect(app: &str) -> Result<InstConn, i32> {
    let path = inst_path(app)?;
    let (address, length) = unix_addr(&path)?;
    let conn = InstConn { fd: new_socket(false)? };
    // SAFETY: the address record and its length are valid for the call.
    if unsafe { connect(conn.fd, address.as_ptr(), length) } < 0 {
        return Err(ffi::errno());
    }
    if peer_uid(conn.fd) != Some(euid()) {
        return Err(13);
    }
    // SAFETY: fcntl(F_SETFL, O_NONBLOCK) on the own socket.
    unsafe { ffi::fcntl(conn.fd, F_SETFL, ffi::O_NONBLOCK) };
    Ok(conn)
}

impl InstConn {
    /// None: nothing yet; Some(0): the other side closed
    pub(crate) fn read(&mut self, bytes: &mut [u8]) -> Result<Option<usize>, i32> {
        // SAFETY: read receives the exclusive writable range of the caller's slice.
        result(unsafe { ffi::read(self.fd, bytes.as_mut_ptr().cast(), bytes.len()) })
    }

    /// None: the socket buffer is full (never SIGPIPE)
    pub(crate) fn write(&mut self, bytes: &[u8]) -> Result<Option<usize>, i32> {
        // SAFETY: send reads a valid slice on a connected socket (SO_NOSIGPIPE).
        result(unsafe { send(self.fd, bytes.as_ptr().cast(), bytes.len(), 0) })
    }
}

impl Drop for InstConn {
    fn drop(&mut self) {
        close(self.fd);
    }
}
