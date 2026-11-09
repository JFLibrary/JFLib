// macOS and iOS child processes through posix_spawn (docs/APPLE.md): no fork in a process with threads.
// A child gets its pipes on 0, 1 and 2, a process group of its own (killed as one, as on Linux),
// SIGPIPE back at its default and no blocked signals, and none of this process's other descriptors
// (POSIX_SPAWN_CLOEXEC_DEFAULT).  posix_spawn reports a failed exec itself.  iOS allows no child
// processes: there every start fails with the system's error.
use alloc::{format, string::String, vec::Vec};
use core::ffi::{c_char, c_int, c_void};
use super::Command;

type SpawnObj = *mut c_void; // posix_spawn_file_actions_t / posix_spawnattr_t: pointers on Darwin

#[link(name = "System")]
unsafe extern "C" {
    fn posix_spawn(pid: *mut c_int, path: *const c_char, actions: *const SpawnObj, attr: *const SpawnObj,
                   argv: *const *const c_char, envp: *const *const c_char) -> c_int;
    fn posix_spawn_file_actions_init(a: *mut SpawnObj) -> c_int;
    fn posix_spawn_file_actions_destroy(a: *mut SpawnObj) -> c_int;
    fn posix_spawn_file_actions_adddup2(a: *mut SpawnObj, fd: c_int, new: c_int) -> c_int;
    fn posix_spawn_file_actions_addopen(a: *mut SpawnObj, fd: c_int, path: *const c_char, flags: c_int, mode: u16) -> c_int;
    fn posix_spawn_file_actions_addchdir_np(a: *mut SpawnObj, path: *const c_char) -> c_int;
    fn posix_spawnattr_init(a: *mut SpawnObj) -> c_int;
    fn posix_spawnattr_destroy(a: *mut SpawnObj) -> c_int;
    fn posix_spawnattr_setflags(a: *mut SpawnObj, flags: i16) -> c_int;
    fn posix_spawnattr_setsigdefault(a: *mut SpawnObj, set: *const u32) -> c_int;
    fn posix_spawnattr_setsigmask(a: *mut SpawnObj, set: *const u32) -> c_int;
    fn execve(path: *const c_char, argv: *const *const c_char, envp: *const *const c_char) -> c_int;
    fn waitid(idtype: c_int, id: u32, info: *mut u8, options: c_int) -> c_int;
    fn waitpid(pid: c_int, status: *mut c_int, options: c_int) -> c_int;
    fn kill(pid: c_int, sig: c_int) -> c_int;
    fn geteuid() -> u32;
    fn access(path: *const c_char, mode: c_int) -> c_int;
    fn signal(sig: c_int, handler: usize) -> usize;
}

const POSIX_SPAWN_SETPGROUP: i16 = 0x0002;
const POSIX_SPAWN_SETSIGDEF: i16 = 0x0004;
const POSIX_SPAWN_SETSIGMASK: i16 = 0x0008;
const POSIX_SPAWN_SETSID: i16 = 0x0400;
const POSIX_SPAWN_CLOEXEC_DEFAULT: i16 = 0x4000;
const SIGPIPE: c_int = 13;
const SIGKILL: c_int = 9;
const SIG_DFL: usize = 0;
const SIG_IGN: usize = 1;
const F_SETFD: c_int = 2;
const F_SETFL: c_int = 4;
const F_DUPFD_CLOEXEC: c_int = 67;
const FD_CLOEXEC: c_int = 1;
const O_NONBLOCK: c_int = 4;
const O_RDWR: c_int = 2;
const P_PID: c_int = 1;
const WEXITED: c_int = 0x04;
const WNOHANG: c_int = 0x01;
const WNOWAIT: c_int = 0x20;
const EINTR: i32 = 4;
const EAGAIN: i32 = 35;
const X_OK: c_int = 1;

use crate::apple::ffi;

struct Fd(i32);
impl Drop for Fd {
    fn drop(&mut self) {
        // SAFETY: exclusively owned descriptor.
        if self.0 >= 0 { unsafe { ffi::close(self.0); } }
    }
}
fn pipe() -> Result<(Fd, Fd), u32> {
    let mut pair = [-1i32; 2];
    // SAFETY: pipe writes two descriptors; both are made close-on-exec at once.
    if unsafe { ffi::pipe(pair.as_mut_ptr()) } < 0 { return Err(ffi::errno() as u32); }
    let mut ends = (Fd(pair[0]), Fd(pair[1]));
    for end in [&mut ends.0, &mut ends.1] {
        // SAFETY: fcntl on an owned pipe end; a pipe on 0..2 moves away from stdio (F_DUPFD_CLOEXEC from 3).
        unsafe {
            ffi::fcntl(end.0, F_SETFD, FD_CLOEXEC);
            if end.0 < 3 {
                let replacement = ffi::fcntl(end.0, F_DUPFD_CLOEXEC, 3);
                if replacement < 0 { return Err(ffi::errno() as u32); }
                *end = Fd(replacement);
            }
        }
    }
    Ok(ends)
}
fn z(s: &str) -> Vec<u8> { let mut v = s.as_bytes().to_vec(); v.push(0); v }
fn nonblock(fd: &Fd) {
    // SAFETY: fcntl operates on an owned pipe endpoint.
    unsafe { ffi::fcntl(fd.0, F_SETFL, O_NONBLOCK); }
}
/// Native child with independent output pipes; drop terminates its process group.
pub struct Child {
    pid: i32, input: Option<Fd>, output: Fd, error: Fd, status: Option<i32>, reaped: bool,
}
// the file of a program: a path as it is, a name looked up in PATH -- the first executable file
// (ENOENT when missing)
fn resolve(program: &str) -> Result<String, u32> {
    if program.contains('/') { return Ok(String::from(program)); }
    let executable = |p: &String| {
        let c = z(p);
        // SAFETY: access(path, X_OK) on a NUL-terminated path.
        crate::file_exists_raw(p) == 1 && unsafe { access(c.as_ptr().cast(), X_OK) } == 0
    };
    crate::env("PATH").unwrap_or_else(|| String::from("/usr/bin:/bin")).split(':')
        .filter(|p| !p.is_empty())
        .map(|p| format!("{}/{}", p, program))
        .find(executable).ok_or(2u32)
}
// this process's environment with the overrides, NUL-terminated entries
fn environment(env: &[(&str, Option<&str>)]) -> Vec<Vec<u8>> {
    let mut envs = crate::apple::process_environment();
    for (key, value) in env {
        let prefix = format!("{key}="); envs.retain(|v| !v.starts_with(prefix.as_bytes()));
        if let Some(v) = value { envs.push(format!("{prefix}{v}").into_bytes()); }
    }
    for s in &mut envs { s.push(0); }
    envs
}

/// what a posix_spawn needs, freed on drop
struct Spawn { actions: SpawnObj, attr: SpawnObj }
impl Spawn {
    fn new(flags: i16) -> Result<Spawn, u32> {
        let mut s = Spawn { actions: core::ptr::null_mut(), attr: core::ptr::null_mut() };
        let pipe_bit: u32 = 1 << (SIGPIPE - 1);
        let none: u32 = 0;
        // SAFETY: init writes the two objects; the signal sets live during the calls.
        unsafe {
            let e = posix_spawn_file_actions_init(&mut s.actions);
            if e != 0 { return Err(e as u32); }
            let e = posix_spawnattr_init(&mut s.attr);
            if e != 0 { return Err(e as u32); }
            posix_spawnattr_setflags(&mut s.attr, flags | POSIX_SPAWN_SETSIGDEF | POSIX_SPAWN_SETSIGMASK | POSIX_SPAWN_CLOEXEC_DEFAULT);
            posix_spawnattr_setsigdefault(&mut s.attr, &pipe_bit);
            posix_spawnattr_setsigmask(&mut s.attr, &none);
        }
        Ok(s)
    }
    fn run(&mut self, exe: &[u8], argv: &[usize], envp: &[usize]) -> Result<i32, u32> {
        let mut pid = 0;
        // SAFETY: NUL-terminated path, 0-terminated pointer arrays of NUL-terminated strings, both
        // objects initialized.
        let e = unsafe { posix_spawn(&mut pid, exe.as_ptr().cast(), &self.actions, &self.attr, argv.as_ptr().cast(), envp.as_ptr().cast()) };
        if e != 0 { Err(e as u32) } else { Ok(pid) }
    }
}
impl Drop for Spawn {
    fn drop(&mut self) {
        // SAFETY: both objects were initialized by new (a null one is skipped).
        unsafe {
            if !self.actions.is_null() { posix_spawn_file_actions_destroy(&mut self.actions); }
            if !self.attr.is_null() { posix_spawnattr_destroy(&mut self.attr); }
        }
    }
}

fn argv_of(program: &str, args: &[&str]) -> (Vec<Vec<u8>>, Vec<usize>) {
    let strs: Vec<Vec<u8>> = core::iter::once(program).chain(args.iter().copied()).map(z).collect();
    let mut argv: Vec<usize> = strs.iter().map(|s| s.as_ptr() as usize).collect(); argv.push(0);
    (strs, argv)
}

pub(super) fn spawn(c: &Command<'_>) -> Result<Child, u32> {
    let program = resolve(c.program)?;
    let exe = z(&program); let dir = c.cwd.map(z);
    let (_strs, argv) = argv_of(c.program, c.args);
    let envs = environment(c.env);
    let mut envp: Vec<usize> = envs.iter().map(|s| s.as_ptr() as usize).collect(); envp.push(0);
    let (ir, iw) = pipe()?; let (or, ow) = pipe()?; let (er, ew) = pipe()?;
    let mut sp = Spawn::new(POSIX_SPAWN_SETPGROUP)?; // process group 0: the child's own
    // SAFETY: file actions on the initialized object; the descriptors and the path live until run.
    unsafe {
        for (fd, dst) in [(ir.0, 0), (ow.0, 1), (ew.0, 2)] {
            posix_spawn_file_actions_adddup2(&mut sp.actions, fd, dst);
        }
        if let Some(d) = &dir { posix_spawn_file_actions_addchdir_np(&mut sp.actions, d.as_ptr().cast()); }
    }
    let pid = sp.run(&exe, &argv, &envp)?;
    drop(ir); drop(ow); drop(ew);
    nonblock(&iw); nonblock(&or); nonblock(&er);
    Ok(Child { pid, input: Some(iw), output: or, error: er, status: None, reaped: false })
}
fn read(fd: &Fd, bytes: &mut [u8]) -> Result<Option<usize>, u32> {
    if bytes.is_empty() { return Ok(None); } // no room: neither data nor EOF
    loop {
        // SAFETY: writable slice lives for the call, descriptor is owned.
        let r = unsafe { ffi::read(fd.0, bytes.as_mut_ptr().cast(), bytes.len()) };
        if r >= 0 { return Ok(Some(r as usize)); }
        match ffi::errno() { EINTR => continue, EAGAIN => return Ok(None), e => return Err(e as u32) }
    }
}
impl Child {
    /// Read stdout without waiting; None means no data, Some(0) means EOF.
    pub fn read_stdout(&mut self, bytes: &mut [u8]) -> Result<Option<usize>, u32> { read(&self.output, bytes) }
    /// Read stderr without waiting; None means no data, Some(0) means EOF.
    pub fn read_stderr(&mut self, bytes: &mut [u8]) -> Result<Option<usize>, u32> { read(&self.error, bytes) }
    /// Write available bytes; retry None or the unwritten suffix.
    pub fn write_stdin(&mut self, bytes: &[u8]) -> Result<Option<usize>, u32> {
        let fd = self.input.as_ref().ok_or(32u32)?;
        // SAFETY: borrowed buffer and owned fd; plat's start ignores SIGPIPE (apple.rs).
        let r = unsafe { ffi::write(fd.0, bytes.as_ptr().cast(), bytes.len()) };
        if r >= 0 { return Ok(Some(r as usize)); }
        match ffi::errno() { EAGAIN | EINTR => Ok(None), e => Err(e as u32) }
    }
    /// Close stdin to request orderly helper shutdown.
    pub fn close_stdin(&mut self) { self.input = None; }
    /// Poll exit status; signals map to 128 + signal.
    pub fn try_wait(&mut self) -> Result<Option<i32>, u32> {
        if self.status.is_some() { return Ok(self.status); }
        let mut info = [0u8; 104]; // Darwin's siginfo_t: code at 8, pid at 12, status at 20
        // SAFETY: waitid(P_PID, ..., WEXITED | WNOHANG | WNOWAIT) keeps the zombie, reserving its
        // PID and group until Drop.
        let r = unsafe { waitid(P_PID, self.pid as u32, info.as_mut_ptr(), WEXITED | WNOHANG | WNOWAIT) };
        if r < 0 { return Err(ffi::errno() as u32); }
        let word = |o: usize| i32::from_ne_bytes([info[o], info[o + 1], info[o + 2], info[o + 3]]);
        if word(12) != 0 {
            let status = word(20);
            self.status = Some(if word(8) == 1 { status } else { 128 + status }); // CLD_EXITED, else the signal
        }
        Ok(self.status)
    }
    /// Terminate the owned process group and reap its leader.
    pub fn terminate(&mut self) {
        if self.reaped { return; }
        // SAFETY: a negative pid addresses the child's own group (POSIX_SPAWN_SETPGROUP), never ours.
        unsafe { kill(-self.pid, SIGKILL); }
        let mut status = 0;
        // SAFETY: reap only this child, retry EINTR; its PID cannot have been reused (the zombie stayed).
        while unsafe { waitpid(self.pid, &mut status, 0) } < 0 && ffi::errno() == EINTR {}
        if self.status.is_none() { self.status = Some(137); }
        self.reaped = true;
    }
}
impl Drop for Child { fn drop(&mut self) { self.terminate(); } }
pub(super) fn elevated() -> bool {
    // SAFETY: no arguments.
    unsafe { geteuid() == 0 }
}
/// execve in place of this process; SIGPIPE back to SIG_DFL first (an ignored signal would stay
/// ignored in the new program) -> the error number when it failed
pub(super) fn exec(program: &str, args: &[&str], env: &[(&str, Option<&str>)]) -> u32 {
    let path = match resolve(program) { Ok(p) => p, Err(e) => return e };
    let exe = z(&path);
    let (_strs, argv) = argv_of(program, args);
    let envs = environment(env);
    let mut envp: Vec<usize> = envs.iter().map(|s| s.as_ptr() as usize).collect(); envp.push(0);
    // SAFETY: signal with a constant handler; execve with the NUL-terminated strings and 0-terminated
    // pointer arrays above, alive during the call.
    unsafe {
        signal(SIGPIPE, SIG_DFL);
        execve(exe.as_ptr().cast(), argv.as_ptr().cast(), envp.as_ptr().cast());
        let e = ffi::errno();
        signal(SIGPIPE, SIG_IGN); // the failed exec left this process running: ignored again as at start-up
        e as u32
    }
}
/// The file of a program name as resolve finds it.
pub(super) fn find(name: &str) -> Option<String> { resolve(name).ok() }
/// A program on its own (process::launch): a session of its own (POSIX_SPAWN_SETSID), standard
/// handles on /dev/null; a thread of this process reaps it when it ends (after this process ended,
/// launchd does).
pub(super) fn launch(program: &str, args: &[&str]) -> Result<(), u32> {
    let path = resolve(program)?;
    let exe = z(&path); let null = z("/dev/null");
    let (_strs, argv) = argv_of(program, args);
    let envs = environment(&[]);
    let mut envp: Vec<usize> = envs.iter().map(|s| s.as_ptr() as usize).collect(); envp.push(0);
    let mut sp = Spawn::new(POSIX_SPAWN_SETSID)?;
    // SAFETY: file actions on the initialized object; the path lives until run.
    unsafe {
        for fd in 0..3 { posix_spawn_file_actions_addopen(&mut sp.actions, fd, null.as_ptr().cast(), O_RDWR, 0); }
    }
    let pid = sp.run(&exe, &argv, &envp)?;
    let reaper = crate::thread_spawn(16 * 1024, alloc::boxed::Box::new(move || {
        let mut status = 0;
        // SAFETY: reap this one child, retry EINTR.
        while unsafe { waitpid(pid, &mut status, 0) } < 0 && ffi::errno() == EINTR {}
    }));
    if let Some(t) = reaper { t.detach(); } // nobody joins it
    Ok(())
}
/// as root: the program itself; otherwise not yet (macOS has no pkexec: an authorization dialog
/// comes later, docs/APPLE.md AP7)
pub(super) fn run_elevated(program: &str, args: &[&str]) -> Result<i32, String> {
    if !elevated() {
        return Err(format!("Administratorrechte nötig: sudo {} {}", program, args.join(" ")));
    }
    let c = Command { program, args, cwd: None, env: &[] };
    let mut child = spawn(&c).map_err(|e| format!("{} startet nicht (Fehler {})", program, e))?;
    let mut b = [0u8; 4096];
    loop {
        while let Ok(Some(n)) = child.read_stdout(&mut b) { if n == 0 { break; } }
        while let Ok(Some(n)) = child.read_stderr(&mut b) { if n == 0 { break; } }
        match child.try_wait() {
            Ok(Some(code)) => return Ok(code),
            Ok(None) => crate::sleep_ms(100),
            Err(e) => return Err(format!("Fehler {}", e)),
        }
    }
}
