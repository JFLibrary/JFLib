// Linux child pipes, exec and owned process groups. All allocations precede fork.
use alloc::{format, string::String, vec::Vec};
use rt::os::{syscall3, syscall6};
use super::Command;

struct Fd(i32);
impl Drop for Fd {
    fn drop(&mut self) {
        // SAFETY: exclusively owned descriptor.
        if self.0 >= 0 { unsafe { syscall3(3, self.0 as usize, 0, 0); } }
    }
}
fn pipe() -> Result<(Fd, Fd), u32> {
    let mut pair = [-1i32; 2];
    // SAFETY: pipe2 writes two descriptors, closed on exec unless dup2'd.
    let r = unsafe { syscall3(293, pair.as_mut_ptr() as usize, 0x80000, 0) };
    if r < 0 { return Err((-r) as u32); }
    let mut ends = (Fd(pair[0]), Fd(pair[1]));
    for end in [&mut ends.0, &mut ends.1] {
        if end.0 < 3 {
            // SAFETY: move a pipe away from stdio; F_DUPFD_CLOEXEC starts at fd 3.
            let replacement = unsafe { syscall3(72, end.0 as usize, 1030, 3) };
            if replacement < 0 { return Err((-replacement) as u32); }
            *end = Fd(replacement as i32);
        }
    }
    Ok(ends)
}
fn z(s: &str) -> Vec<u8> { let mut v = s.as_bytes().to_vec(); v.push(0); v }
fn nonblock(fd: &Fd) {
    // SAFETY: fcntl operates on an owned pipe endpoint.
    unsafe { syscall3(72, fd.0 as usize, 4, 0x800); }
}
/// Native child with independent output pipes; drop terminates its process group.
pub struct Child {
    pid: i32, input: Option<Fd>, output: Fd, error: Fd, status: Option<i32>, reaped: bool,
}
// the file of a program: a path as it is, a name looked up in PATH -- the first executable
// file, as execvp takes it (ENOENT when missing)
fn resolve(program: &str) -> Result<String, u32> {
    if program.contains('/') { return Ok(String::from(program)); }
    let executable = |p: &String| {
        let c = z(p);
        // SAFETY: access(path, X_OK) on a NUL-terminated path.
        crate::file_exists_raw(p) == 1 && unsafe { syscall3(21, c.as_ptr() as usize, 1, 0) } == 0
    };
    crate::env("PATH").unwrap_or_else(|| String::from("/usr/bin:/bin")).split(':')
        .filter(|p| !p.is_empty())
        .map(|p| format!("{}/{}", p, program))
        .find(executable).ok_or(2u32)
}
// this process's environment with the overrides, NUL-terminated entries
fn environment(env: &[(&str, Option<&str>)]) -> Vec<Vec<u8>> {
    let mut envs = crate::linux::process_environment();
    for (key, value) in env {
        let prefix = format!("{key}="); envs.retain(|v| !v.starts_with(prefix.as_bytes()));
        if let Some(v) = value { envs.push(format!("{prefix}{v}").into_bytes()); }
    }
    for s in &mut envs { s.push(0); }
    envs
}
pub(super) fn spawn(c: &Command<'_>) -> Result<Child, u32> {
    let program = resolve(c.program)?;
    let exe = z(&program); let dir = c.cwd.map(z);
    let strs: Vec<Vec<u8>> = core::iter::once(c.program).chain(c.args.iter().copied()).map(z).collect();
    let mut argv: Vec<usize> = strs.iter().map(|s| s.as_ptr() as usize).collect(); argv.push(0);
    let envs = environment(c.env);
    let mut envp: Vec<usize> = envs.iter().map(|s| s.as_ptr() as usize).collect(); envp.push(0);
    let (ir, iw) = pipe()?; let (or, ow) = pipe()?; let (er, ew) = pipe()?;
    let (failr, failw) = pipe()?;
    let (dfl, nomask) = ([0usize; 4], [0u64; 1]); // SIG_DFL, the empty signal set (made before the fork)
    // SAFETY: fork child uses only syscalls and preallocated buffers until exec/exit.
    let pid = unsafe {
        let pid = syscall3(57, 0, 0, 0);
        if pid == 0 {
            let mut failure = syscall3(112, 0, 0, 0);
            if failure >= 0 {
                for (fd, dst) in [(ir.0, 0), (ow.0, 1), (ew.0, 2)] {
                    let r = syscall3(33, fd as usize, dst, 0); if r < 0 { failure = r; break; }
                }
            }
            if failure >= 0 { if let Some(d) = &dir { failure = syscall3(80, d.as_ptr() as usize, 0, 0); } }
            // close_range(3, ~0, CLOSE_RANGE_CLOEXEC): no other descriptor reaches the program (old kernels: ignored)
            if failure >= 0 { syscall3(436, 3, 0xFFFF_FFFF, 4); }
            // SIGPIPE back to SIG_DFL and no blocked signals: an ignored or blocked signal
            // would stay so in the program (a producer in `a | head` would not end)
            if failure >= 0 {
                syscall6(13, 13, dfl.as_ptr() as usize, 0, 8, 0, 0); // rt_sigaction(SIGPIPE, SIG_DFL)
                syscall6(14, 2, nomask.as_ptr() as usize, 0, 8, 0, 0); // rt_sigprocmask(SIG_SETMASK, {})
            }
            if failure >= 0 { failure = syscall3(59, exe.as_ptr() as usize, argv.as_ptr() as usize, envp.as_ptr() as usize); }
            let code = (-failure) as u32;
            syscall3(1, failw.0 as usize, &code as *const u32 as usize, 4);
            syscall3(231, 127, 0, 0);
            loop { core::hint::spin_loop(); }
        }
        pid
    };
    if pid < 0 { return Err((-pid) as u32); }
    drop(ir); drop(ow); drop(ew); drop(failw);
    let mut error = 0u32;
    // SAFETY: successful exec closes the error pipe; errors write exactly one u32.
    let n = loop { let r = unsafe { syscall3(0, failr.0 as usize, &mut error as *mut u32 as usize, 4) }; if r != -4 { break r; } };
    nonblock(&iw); nonblock(&or); nonblock(&er);
    let mut child = Child { pid: pid as i32, input: Some(iw), output: or, error: er, status: None, reaped: false };
    if n != 0 { child.terminate(); return Err(if error == 0 { 5 } else { error }); }
    Ok(child)
}
fn read(fd: &Fd, bytes: &mut [u8]) -> Result<Option<usize>, u32> {
    if bytes.is_empty() { return Ok(None); } // no room: neither data nor EOF
    loop {
        // SAFETY: writable slice lives for the syscall, descriptor is owned.
        let r = unsafe { syscall3(0, fd.0 as usize, bytes.as_mut_ptr() as usize, bytes.len()) };
        if r == -4 { continue; }
        return if r == -11 { Ok(None) } else if r < 0 { Err((-r) as u32) } else { Ok(Some(r as usize)) };
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
        // SAFETY: borrowed buffer and owned fd; platform startup ignores SIGPIPE.
        let r = unsafe { syscall3(1, fd.0 as usize, bytes.as_ptr() as usize, bytes.len()) };
        if r == -11 || r == -4 { Ok(None) } else if r < 0 { Err((-r) as u32) } else { Ok(Some(r as usize)) }
    }
    /// Close stdin to request orderly helper shutdown.
    pub fn close_stdin(&mut self) { self.input = None; }
    /// Poll exit status; signals map to 128 + signal.
    pub fn try_wait(&mut self) -> Result<Option<i32>, u32> {
        if self.status.is_some() { return Ok(self.status); }
        let mut info = [0u64; 16];
        // SAFETY: x86-64 siginfo_t is 128 bytes. waitid(P_PID, ..., WEXITED |
        // WNOHANG | WNOWAIT) retains the zombie, reserving its PID/group until Drop.
        let r = unsafe { syscall6(247, 1, self.pid as usize, info.as_mut_ptr() as usize, 0x1000005, 0, 0) };
        if r < 0 { return Err((-r) as u32); }
        if info[2] as u32 != 0 {
            let code = info[1] as u32;
            let status = info[3] as u32 as i32;
            self.status = Some(if code == 1 { status } else { 128 + status });
        }
        Ok(self.status)
    }
    /// Terminate the owned process group and reap its leader.
    pub fn terminate(&mut self) {
        if self.reaped { return; }
        // SAFETY: negative pid addresses the group created by setsid, never our group.
        unsafe { syscall3(62, (-self.pid) as isize as usize, 9, 0); }
        let mut status = 0i32;
        // SAFETY: reap only this child, retry EINTR. Its PID cannot have been reused.
        while unsafe { syscall6(61, self.pid as usize, &mut status as *mut i32 as usize, 0, 0, 0, 0) } == -4 {}
        if self.status.is_none() { self.status = Some(137); }
        self.reaped = true;
    }
}
impl Drop for Child { fn drop(&mut self) { self.terminate(); } }
pub(super) fn elevated() -> bool {
    // SAFETY: geteuid takes no arguments.
    unsafe { syscall3(107, 0, 0, 0) == 0 }
}
/// execve in place of this process; SIGPIPE back to SIG_DFL first (an ignored signal would
/// stay ignored in the new program) -> the error number when it failed
pub(super) fn exec(program: &str, args: &[&str], env: &[(&str, Option<&str>)]) -> u32 {
    let path = match resolve(program) { Ok(p) => p, Err(e) => return e };
    let exe = z(&path);
    let strs: Vec<Vec<u8>> = core::iter::once(program).chain(args.iter().copied()).map(z).collect();
    let mut argv: Vec<usize> = strs.iter().map(|s| s.as_ptr() as usize).collect(); argv.push(0);
    let envs = environment(env);
    let mut envp: Vec<usize> = envs.iter().map(|s| s.as_ptr() as usize).collect(); envp.push(0);
    let dfl = [0usize; 4];
    // SAFETY: rt_sigaction(SIGPIPE, SIG_DFL) with a kernel sigaction of 4 words; execve with
    // the NUL-terminated strings and 0-terminated pointer arrays above, alive during the call.
    let r = unsafe {
        syscall6(13, 13, dfl.as_ptr() as usize, 0, 8, 0, 0);
        syscall3(59, exe.as_ptr() as usize, argv.as_ptr() as usize, envp.as_ptr() as usize)
    };
    let ign = [1usize, 0, 0, 0];
    // SAFETY: the failed exec left this process running: SIGPIPE ignored again as at start-up
    // (SIG_IGN needs no restorer).
    unsafe { syscall6(13, 13, ign.as_ptr() as usize, 0, 8, 0, 0); }
    (-r) as u32
}
/// The file of a program name as resolve finds it.
pub(super) fn find(name: &str) -> Option<String> { resolve(name).ok() }
/// A program on its own (process::launch): forked twice -- the middle process starts a session
/// and ends at once, so init adopts the program and no signal to this process group reaches
/// it; standard handles on /dev/null, signals as spawn leaves them.  The exec's error number
/// comes back through a pipe that a successful exec closes.
pub(super) fn launch(program: &str, args: &[&str]) -> Result<(), u32> {
    let path = resolve(program)?;
    let exe = z(&path); let null = z("/dev/null");
    let strs: Vec<Vec<u8>> = core::iter::once(program).chain(args.iter().copied()).map(z).collect();
    let mut argv: Vec<usize> = strs.iter().map(|s| s.as_ptr() as usize).collect(); argv.push(0);
    let envs = environment(&[]);
    let mut envp: Vec<usize> = envs.iter().map(|s| s.as_ptr() as usize).collect(); envp.push(0);
    let (failr, failw) = pipe()?;
    let (dfl, nomask) = ([0usize; 4], [0u64; 1]); // SIG_DFL, the empty signal set (made before the fork)
    // SAFETY: both forked processes use only syscalls and the buffers made before the fork until
    // exec / exit.
    let pid = unsafe {
        let pid = syscall3(57, 0, 0, 0);
        if pid == 0 {
            let mut failure = syscall3(112, 0, 0, 0); // setsid: no terminal, no group of ours
            if failure >= 0 {
                let grand = syscall3(57, 0, 0, 0);
                if grand == 0 {
                    let fd = syscall3(2, null.as_ptr() as usize, 2, 0); // open("/dev/null", O_RDWR)
                    if fd >= 0 {
                        for dst in 0..3 { syscall3(33, fd as usize, dst, 0); }
                        if fd > 2 { syscall3(3, fd as usize, 0, 0); }
                    }
                    syscall3(436, 3, 0xFFFF_FFFF, 4); // close_range(3, ~0, CLOSE_RANGE_CLOEXEC)
                    syscall6(13, 13, dfl.as_ptr() as usize, 0, 8, 0, 0); // rt_sigaction(SIGPIPE, SIG_DFL)
                    syscall6(14, 2, nomask.as_ptr() as usize, 0, 8, 0, 0); // rt_sigprocmask(SIG_SETMASK, {})
                    failure = syscall3(59, exe.as_ptr() as usize, argv.as_ptr() as usize, envp.as_ptr() as usize);
                    let code = (-failure) as u32;
                    syscall3(1, failw.0 as usize, &code as *const u32 as usize, 4);
                    syscall3(231, 127, 0, 0);
                    loop { core::hint::spin_loop(); }
                }
                failure = grand.min(0);
            }
            if failure < 0 {
                let code = (-failure) as u32;
                syscall3(1, failw.0 as usize, &code as *const u32 as usize, 4);
            }
            syscall3(231, 0, 0, 0);
            loop { core::hint::spin_loop(); }
        }
        pid
    };
    if pid < 0 { return Err((-pid) as u32); }
    drop(failw);
    let mut status = 0i32;
    // SAFETY: reap the middle process (it ends at once), retry EINTR.
    while unsafe { syscall6(61, pid as usize, &mut status as *mut i32 as usize, 0, 0, 0, 0) } == -4 {}
    let mut error = 0u32;
    // SAFETY: a successful exec closes the error pipe; a failure writes exactly one u32.
    let n = loop { let r = unsafe { syscall3(0, failr.0 as usize, &mut error as *mut u32 as usize, 4) }; if r != -4 { break r; } };
    if n != 0 { return Err(if error == 0 { 5 } else { error }); }
    Ok(())
}
/// as root: the program itself; otherwise pkexec (polkit asks for the password)
pub(super) fn run_elevated(program: &str, args: &[&str]) -> Result<i32, String> {
    let mut all: Vec<&str> = Vec::new();
    // pkexec only from the system directories, never through the caller's PATH
    let mut pkexec = false;
    let tool = if elevated() {
        program
    } else {
        let Some(found) = ["/usr/bin/pkexec", "/bin/pkexec"].into_iter().find(|p| crate::file_exists_raw(p) == 1) else {
            return Err(format!("Root-Rechte nötig: sudo {} {}", program, args.join(" ")));
        };
        all.push(program);
        pkexec = true;
        found
    };
    all.extend_from_slice(args);
    let c = Command { program: tool, args: &all, cwd: None, env: &[] };
    let mut child = spawn(&c).map_err(|e| format!("{} startet nicht (Fehler {})", tool, e))?;
    let mut b = [0u8; 4096];
    loop {
        while let Ok(Some(n)) = child.read_stdout(&mut b) { if n == 0 { break; } }
        while let Ok(Some(n)) = child.read_stderr(&mut b) { if n == 0 { break; } }
        match child.try_wait() {
            // pkexec's own codes (a program that returns them itself looks the same)
            Ok(Some(126)) if pkexec => return Err(String::from("Die Rechteabfrage wurde abgelehnt")),
            Ok(Some(127)) if pkexec => return Err(format!("pkexec konnte {} nicht ausführen", program)),
            Ok(Some(code)) => return Ok(code),
            Ok(None) => crate::sleep_ms(100),
            Err(e) => return Err(format!("Fehler {}", e)),
        }
    }
}
