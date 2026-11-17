// Nonblocking Unix sockets restricted to the effective user. Linux raw syscalls.
// New code; socket paths never accept external directory or filename input.

use alloc::{format, string::String};
use rt::os::{syscall6, AT_FDCWD, O_CLOEXEC, O_DIRECTORY, O_NOFOLLOW, O_NONBLOCK, SYS_ACCEPT4, SYS_BIND, SYS_CLOSE, SYS_CONNECT, SYS_FCHMODAT, SYS_FCNTL, SYS_FSTAT, SYS_GETEUID, SYS_GETPID, SYS_GETSOCKOPT, SYS_LISTEN, SYS_MKDIRAT, SYS_NEWFSTATAT, SYS_OPENAT, SYS_READ, SYS_SENDTO, SYS_SOCKET, SYS_UNLINKAT, Stat};

const EAGAIN: isize = -11;

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
        if app.is_empty()
            || app.len() > 32
            || !app
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
        {
            return Err(22);
        }
        // SAFETY: geteuid/getpid take no pointers or process-modifying arguments.
        let (uid, pid) = unsafe {
            (
                syscall6(SYS_GETEUID, 0, 0, 0, 0, 0, 0),
                syscall6(SYS_GETPID, 0, 0, 0, 0, 0, 0),
            )
        };
        // $XDG_RUNTIME_DIR (the user's own, made at login: no other user can take the name
        // first) when it is a plain absolute path, else /tmp/jfgui-agent-<uid> (another user may
        // make that one first: then there is no listener, Err(13))
        let directory = match crate::env("XDG_RUNTIME_DIR").filter(|d| d.starts_with('/') && d.len() <= 48 && !d.split('/').any(|c| c == "..")) {
            Some(d) => format!("{}/jfgui-agent", d.trim_end_matches('/')),
            None => format!("/tmp/jfgui-agent-{uid}"),
        };
        let dir_z = format!("{directory}\0");
        // SAFETY: mkdirat/openat receive a valid terminated path; flags forbid following a symlink.
        let dir_fd = unsafe {
            let mkdir = syscall6(SYS_MKDIRAT, AT_FDCWD, dir_z.as_ptr() as usize, 0o700, 0, 0, 0);
            if mkdir < 0 && mkdir != -17 {
                return Err((-mkdir) as i32);
            }
            syscall6(
                SYS_OPENAT,
                AT_FDCWD,
                dir_z.as_ptr() as usize,
                O_DIRECTORY | O_CLOEXEC | O_NOFOLLOW,
                0,
                0,
                0,
            )
        };
        if dir_fd < 0 {
            return Err((-dir_fd) as i32);
        }
        let mut stat = Stat::default();
        // SAFETY: fstat writes exactly the kernel's stat record to this buffer.
        let status =
            unsafe { syscall6(SYS_FSTAT, dir_fd as usize, &mut stat as *mut Stat as usize, 0, 0, 0, 0) };
        close(dir_fd as i32);
        if status < 0 {
            return Err((-status) as i32);
        }
        if stat.st_uid != uid as u32 || stat.st_mode & 0o170777 != 0o040700 {
            return Err(13);
        }
        let path = format!("{directory}/{app}-{pid}-{}.sock", crate::time_us());
        if path.len() >= 108 {
            return Err(36);
        }
        let mut address = [0u8; 110];
        address[..2].copy_from_slice(&1u16.to_ne_bytes());
        address[2..2 + path.len()].copy_from_slice(path.as_bytes());
        // SAFETY: socket has no borrowed memory and requests CLOEXEC/nonblocking from creation.
        let fd = unsafe { syscall6(SYS_SOCKET, 1, 1 | 0x80000 | 0x800, 0, 0, 0, 0) };
        if fd < 0 {
            return Err((-fd) as i32);
        }
        // SAFETY: sockaddr_un is bounded and includes its terminating zero byte.
        let bound = unsafe {
            syscall6(
                SYS_BIND,
                fd as usize,
                address.as_ptr() as usize,
                path.len() + 3,
                0,
                0,
                0,
            )
        };
        if bound < 0 {
            close(fd as i32);
            return Err((-bound) as i32);
        }
        let path_z = format!("{path}\0");
        // SAFETY: the socket lives inside the checked private directory; fchmodat/listen use owned path/fd.
        let status = unsafe {
            let mode = syscall6(SYS_FCHMODAT, AT_FDCWD, path_z.as_ptr() as usize, 0o600, 0, 0, 0);
            if mode < 0 {
                mode
            } else {
                syscall6(SYS_LISTEN, fd as usize, 8, 0, 0, 0, 0)
            }
        };
        if status < 0 {
            close(fd as i32);
            unlink(&path);
            return Err((-status) as i32);
        }
        Ok(Self {
            fd: fd as i32,
            path,
        })
    }
    /// Address to advertise only while control is active.
    pub fn endpoint(&self) -> &str {
        &self.path
    }
    /// Accept at most one available connection and validate its effective UID via SO_PEERCRED.
    pub fn accept(&mut self) -> Result<Option<Stream>, i32> {
        // SAFETY: accept4 is nonblocking and requests no peer address output.
        let fd = unsafe { syscall6(SYS_ACCEPT4, self.fd as usize, 0, 0, 0x80000 | 0x800, 0, 0) };
        if fd == EAGAIN || fd == -4 {
            return Ok(None);
        }
        if fd < 0 {
            return Err((-fd) as i32);
        }
        let mut credentials = [0u32; 3];
        let mut length = 12u32;
        // SAFETY: SO_PEERCRED writes at most the supplied 12-byte ucred record and length.
        let status = unsafe {
            syscall6(
                SYS_GETSOCKOPT,
                fd as usize,
                1,
                17,
                credentials.as_mut_ptr() as usize,
                &mut length as *mut u32 as usize,
                0,
            )
        };
        // SAFETY: geteuid has no arguments.
        let uid = unsafe { syscall6(SYS_GETEUID, 0, 0, 0, 0, 0, 0) };
        if status < 0 || length != 12 || credentials[1] != uid as u32 {
            close(fd as i32);
            return Err(13);
        }
        Ok(Some(Stream { fd: fd as i32 }))
    }
}

impl Stream {
    /// Read available bytes. None means would block; Some(0) means peer EOF.
    pub fn read(&mut self, bytes: &mut [u8]) -> Result<Option<usize>, i32> {
        // SAFETY: read receives the exclusive writable range of the caller's slice.
        result(unsafe {
            syscall6(
                SYS_READ,
                self.fd as usize,
                bytes.as_mut_ptr() as usize,
                bytes.len(),
                0,
                0,
                0,
            )
        })
    }
    /// Write available bytes without SIGPIPE. None means would block.
    pub fn write(&mut self, bytes: &[u8]) -> Result<Option<usize>, i32> {
        // SAFETY: sendto reads a valid slice, uses no destination for a connected socket, and disables SIGPIPE.
        result(unsafe {
            syscall6(
                SYS_SENDTO,
                self.fd as usize,
                bytes.as_ptr() as usize,
                bytes.len(),
                0x4000,
                0,
                0,
            )
        })
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
    if value == EAGAIN || value == -4 {
        Ok(None)
    } else if value < 0 {
        Err((-value) as i32)
    } else {
        Ok(Some(value as usize))
    }
}
fn close(fd: i32) {
    // SAFETY: this module calls close once for each owned descriptor.
    unsafe {
        syscall6(SYS_CLOSE, fd as usize, 0, 0, 0, 0, 0);
    }
}
fn unlink(path: &str) {
    let path = format!("{path}\0");
    // SAFETY: the path was created by this listener and contains no external path components.
    unsafe {
        syscall6(SYS_UNLINKAT, AT_FDCWD, path.as_ptr() as usize, 0, 0, 0, 0);
    }
}

// ---------------------------------------------------------------- the app's single instance
// (instance.rs): the abstract socket "jfgui-instance-<uid>-<app>" (no file, gone with the
// process); the process that binds it first is the app's running instance.  Both sides check
// the other with SO_PEERCRED: a socket of another user under this name is refused, and so is
// a process of this user in another mount namespace -- an abstract name is reachable from
// every sandbox that shares the network namespace (Flatpak with network access, bubblewrap,
// Firejail), and a confined process must not hand the instance files to grant (fs.user) or
// receive a start's arguments.  Snap keeps all processes of a snap in one mount namespace;
// two Flatpak sandboxes of the same app each run alone.

/// the listening side of the instance socket (blocking accept)
pub(crate) struct InstServer {
    fd: i32,
}

/// one exchange over the instance socket (nonblocking reads and writes)
pub(crate) struct InstConn {
    fd: i32,
}

fn euid() -> u32 {
    // SAFETY: geteuid has no arguments.
    unsafe { syscall6(SYS_GETEUID, 0, 0, 0, 0, 0, 0) as u32 }
}

// sockaddr_un with an abstract name (sun_path[0] = 0) and its length
fn inst_addr(app: &str) -> Result<([u8; 110], usize), i32> {
    let name = format!("jfgui-instance-{}-{app}", euid());
    if name.len() > 100 {
        return Err(36);
    }
    let mut address = [0u8; 110];
    address[..2].copy_from_slice(&1u16.to_ne_bytes());
    address[3..3 + name.len()].copy_from_slice(name.as_bytes());
    Ok((address, 3 + name.len()))
}

// the process at the other end of fd runs as this user in this process's mount namespace.
// SO_PEERCRED's pid is 0 for a process outside this pid namespace (refused); SO_PEERPIDFD
// (Linux 6.5) proves afterwards that the pid still names that process, not a later one
fn peer_ok(fd: i32) -> bool {
    let mut credentials = [0u32; 3];
    let mut length = 12u32;
    // SAFETY: SO_PEERCRED writes at most the supplied 12-byte ucred record and length.
    let status = unsafe { syscall6(SYS_GETSOCKOPT, fd as usize, 1, 17, credentials.as_mut_ptr() as usize, &mut length as *mut u32 as usize, 0) };
    if status != 0 || length != 12 || credentials[1] != euid() || credentials[0] == 0 {
        return false;
    }
    let mut pidfd = -1i32;
    let mut length = 4u32;
    // SAFETY: SO_PEERPIDFD writes at most one int and its length (ENOPROTOOPT before 6.5).
    let status = unsafe { syscall6(SYS_GETSOCKOPT, fd as usize, 1, 77, &mut pidfd as *mut i32 as usize, &mut length as *mut u32 as usize, 0) };
    let pidfd = if status == 0 && length == 4 && pidfd >= 0 { Some(pidfd) } else { None };
    let own = mount_ns("/proc/self/ns/mnt");
    let same = own.is_some() && own == mount_ns(&format!("/proc/{}/ns/mnt", credentials[0]));
    let Some(pidfd) = pidfd else { return same };
    // a pidfd turns readable once its process has ended (its pid may then be another's)
    let mut pfd = [pidfd as u32 as u64 | (1u64 << 32)]; // struct pollfd {fd, events POLLIN, revents}
    // SAFETY: one pollfd record, no waiting.
    let alive = unsafe { rt::os::poll(pfd.as_mut_ptr() as usize, 1, 0) } == 0;
    close(pidfd);
    same && alive
}

// (device, inode) of a namespace link
fn mount_ns(path: &str) -> Option<(u64, u64)> {
    let path = format!("{path}\0");
    let mut stat = Stat::default();
    // SAFETY: newfstatat follows the terminated path and writes exactly the kernel's stat record.
    let status = unsafe { syscall6(SYS_NEWFSTATAT, AT_FDCWD, path.as_ptr() as usize, &mut stat as *mut Stat as usize, 0, 0, 0) };
    (status == 0).then_some((stat.st_dev, stat.st_ino))
}

/// bind the instance socket -> None when another process already has it
pub(crate) fn inst_listen(app: &str) -> Result<Option<InstServer>, i32> {
    let (address, length) = inst_addr(app)?;
    // SAFETY: a blocking, close-on-exec Unix stream socket.
    let fd = unsafe { syscall6(SYS_SOCKET, 1, 1 | 0x80000, 0, 0, 0, 0) };
    if fd < 0 {
        return Err((-fd) as i32);
    }
    let fd = fd as i32;
    // SAFETY: the address record and its length are valid for the call.
    let bound = unsafe { syscall6(SYS_BIND, fd as usize, address.as_ptr() as usize, length, 0, 0, 0) };
    if bound == -98 {
        close(fd); // EADDRINUSE: the instance runs
        return Ok(None);
    }
    // SAFETY: listen on the bound socket.
    let status = if bound < 0 { bound } else { unsafe { syscall6(SYS_LISTEN, fd as usize, 4, 0, 0, 0, 0) } };
    if status < 0 {
        close(fd);
        return Err((-status) as i32);
    }
    Ok(Some(InstServer { fd }))
}

impl InstServer {
    /// wait for the next starting process of the same user (blocks)
    pub(crate) fn accept(&self) -> Result<InstConn, i32> {
        loop {
            // SAFETY: accept4 blocks; the new socket is nonblocking and close-on-exec.
            let fd = unsafe { syscall6(SYS_ACCEPT4, self.fd as usize, 0, 0, 0x80000 | 0x800, 0, 0) };
            if fd == -4 {
                continue; // EINTR
            }
            if fd < 0 {
                return Err((-fd) as i32);
            }
            let fd = fd as i32;
            if peer_ok(fd) {
                return Ok(InstConn { fd });
            }
            close(fd);
        }
    }
}

impl Drop for InstServer {
    fn drop(&mut self) {
        close(self.fd);
    }
}

/// connect to the running instance (it must run as the same user)
pub(crate) fn inst_connect(app: &str) -> Result<InstConn, i32> {
    let (address, length) = inst_addr(app)?;
    // SAFETY: a blocking, close-on-exec Unix stream socket.
    let fd = unsafe { syscall6(SYS_SOCKET, 1, 1 | 0x80000, 0, 0, 0, 0) };
    if fd < 0 {
        return Err((-fd) as i32);
    }
    let conn = InstConn { fd: fd as i32 };
    // SAFETY: the address record and its length are valid for the call.
    let status = unsafe { syscall6(SYS_CONNECT, fd as usize, address.as_ptr() as usize, length, 0, 0, 0) };
    if status < 0 {
        return Err((-status) as i32);
    }
    if !peer_ok(conn.fd) {
        return Err(13);
    }
    // SAFETY: fcntl(F_SETFL, O_NONBLOCK) on the own socket.
    unsafe { syscall6(SYS_FCNTL, fd as usize, 4, O_NONBLOCK, 0, 0, 0) };
    Ok(conn)
}

impl InstConn {
    /// None: nothing yet; Some(0): the other side closed
    pub(crate) fn read(&mut self, bytes: &mut [u8]) -> Result<Option<usize>, i32> {
        // SAFETY: read receives the exclusive writable range of the caller's slice.
        result(unsafe { syscall6(SYS_READ, self.fd as usize, bytes.as_mut_ptr() as usize, bytes.len(), 0, 0, 0) })
    }

    /// None: the socket buffer is full (never SIGPIPE)
    pub(crate) fn write(&mut self, bytes: &[u8]) -> Result<Option<usize>, i32> {
        // SAFETY: sendto reads a valid slice on a connected socket, MSG_NOSIGNAL.
        result(unsafe { syscall6(SYS_SENDTO, self.fd as usize, bytes.as_ptr() as usize, bytes.len(), 0x4000, 0, 0) })
    }
}

impl Drop for InstConn {
    fn drop(&mut self) {
        close(self.fd);
    }
}
