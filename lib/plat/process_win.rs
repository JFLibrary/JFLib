// Windows child pipes and job ownership; no console window and only explicit handles inherited.
use alloc::{format, string::String, vec, vec::Vec};
use super::Command;

#[repr(C)]
struct StartupInfo { cb: u32, rest: [u32; 25] }
#[repr(C)]
struct StartupEx { info: StartupInfo, attrs: usize }
#[repr(C)]
struct Security { size: u32, descriptor: usize, inherit: i32 }
#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CreatePipe(read: *mut isize, write: *mut isize, sa: *const Security, size: u32) -> i32;
    fn SetHandleInformation(h: isize, mask: u32, flags: u32) -> i32;
    fn CloseHandle(h: isize) -> i32;
    fn GetLastError() -> u32;
    fn CreateProcessW(app: *const u16, cmd: *mut u16, pa: usize, ta: usize, inherit: i32, flags: u32, env: usize,
        dir: *const u16, si: *const StartupInfo, pi: *mut [isize; 3]) -> i32;
    fn InitializeProcThreadAttributeList(list: usize, count: u32, flags: u32, size: *mut usize) -> i32;
    fn UpdateProcThreadAttribute(list: usize, flags: u32, attr: usize, value: usize, size: usize, old: usize, returned: usize) -> i32;
    fn DeleteProcThreadAttributeList(list: usize);
    fn CreateJobObjectW(sa: usize, name: *const u16) -> isize;
    fn SetInformationJobObject(job: isize, class: u32, info: *const u8, size: u32) -> i32;
    fn AssignProcessToJobObject(job: isize, process: isize) -> i32;
    fn TerminateJobObject(job: isize, code: u32) -> i32;
    fn TerminateProcess(process: isize, code: u32) -> i32;
    fn ResumeThread(thread: isize) -> u32;
    fn GetExitCodeProcess(process: isize, code: *mut u32) -> i32;
    fn WaitForSingleObject(h: isize, ms: u32) -> u32;
    fn PeekNamedPipe(h: isize, buf: usize, size: u32, read: usize, available: *mut u32, left: usize) -> i32;
    fn ReadFile(h: isize, buf: *mut u8, n: u32, done: *mut u32, ov: *mut crate::win::Overlapped) -> i32;
    fn WriteFile(h: isize, buf: *const u8, n: u32, done: *mut u32, ov: *mut crate::win::Overlapped) -> i32;
    fn GetEnvironmentStringsW() -> *mut u16;
    fn FreeEnvironmentStringsW(env: *mut u16) -> i32;
    fn LoadLibraryExA(name: *const u8, file: isize, flags: u32) -> isize;
    fn GetProcAddress(module: isize, name: *const u8) -> usize;
}
/// SHELLEXECUTEINFOW
#[repr(C)]
struct ShellExecuteInfo {
    size: u32, mask: u32, hwnd: isize, verb: *const u16, file: *const u16, params: *const u16, dir: *const u16,
    show: i32, inst_app: isize, id_list: usize, class: *const u16, hkey_class: isize, hot_key: u32, icon: isize, process: isize,
}
#[cfg(target_pointer_width = "64")]
const _: () = assert!(core::mem::size_of::<ShellExecuteInfo>() == 112);
/// a function of a system DLL (0 = missing); only System32 is searched
fn proc_of(dll: &[u8], name: &[u8]) -> usize {
    // SAFETY: NUL-terminated names; the module stays loaded for the process.
    unsafe {
        let m = LoadLibraryExA(dll.as_ptr(), 0, 0x800); // LOAD_LIBRARY_SEARCH_SYSTEM32
        if m == 0 { 0 } else { GetProcAddress(m, name.as_ptr()) }
    }
}
fn wide0(s: &str) -> Vec<u16> { s.encode_utf16().chain(core::iter::once(0)).collect() }
/// ShellExecuteExW "runas", hidden; waits for the program -> its exit code (1223: refused)
pub(super) fn run_elevated(program: &str, args: &[&str]) -> Result<i32, String> {
    let exec = proc_of(b"shell32.dll\0", b"ShellExecuteExW\0");
    if exec == 0 { return Err(String::from("ShellExecuteExW fehlt")); }
    let co = proc_of(b"ole32.dll\0", b"CoInitializeEx\0");
    let couninit = proc_of(b"ole32.dll\0", b"CoUninitialize\0");
    let mut com = false;
    if co != 0 {
        // SAFETY: CoInitializeEx(NULL, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) for the shell on this thread.
        let ci: unsafe extern "system" fn(usize, u32) -> i32 = unsafe { core::mem::transmute(co) };
        // S_OK / S_FALSE need the matching CoUninitialize; RPC_E_CHANGED_MODE leaves COM as it was
        com = unsafe { ci(0, 6) } >= 0;
    }
    let _com = ComGuard(if com { couninit } else { 0 });
    let mut line = String::new();
    for (i, a) in args.iter().enumerate() { if i > 0 { line.push(' '); } line.push_str(&quote(a)); }
    let (verb, file, params) = (wide0("runas"), wide0(program), wide0(&line));
    let mut info = ShellExecuteInfo { size: core::mem::size_of::<ShellExecuteInfo>() as u32, mask: 0x40 | 0x100, hwnd: 0, verb: verb.as_ptr(), file: file.as_ptr(), params: params.as_ptr(),
        dir: core::ptr::null(), show: 0, inst_app: 0, id_list: 0, class: core::ptr::null(), hkey_class: 0, hot_key: 0, icon: 0, process: 0 };
    // SAFETY: ShellExecuteExW(SHELLEXECUTEINFOW*) with live NUL-terminated strings.
    let f: unsafe extern "system" fn(*mut ShellExecuteInfo) -> i32 = unsafe { core::mem::transmute(exec) };
    if unsafe { f(&mut info) } == 0 {
        let e = error();
        return Err(if e == 1223 { String::from("Die Rechteabfrage wurde abgelehnt") } else { format!("Start mit Administratorrechten fehlgeschlagen (Fehler {})", e) });
    }
    if info.process == 0 { return Err(String::from("kein Prozess")); }
    let process = Handle(info.process);
    let mut code = 1u32;
    // SAFETY: owned process handle; INFINITE wait, then its exit code.
    unsafe { WaitForSingleObject(process.0, 0xFFFF_FFFF); GetExitCodeProcess(process.0, &mut code); }
    Ok(code as i32)
}
pub(super) fn elevated() -> bool {
    let p = proc_of(b"shell32.dll\0", b"IsUserAnAdmin\0");
    if p == 0 { return false; }
    // SAFETY: IsUserAnAdmin() -> BOOL.
    let f: unsafe extern "system" fn() -> i32 = unsafe { core::mem::transmute(p) };
    unsafe { f() != 0 }
}
/// Windows cannot replace a process by another program: ERROR_NOT_SUPPORTED
pub(super) fn exec(_program: &str, _args: &[&str], _env: &[(&str, Option<&str>)]) -> u32 { 50 }
struct ComGuard(usize);
impl Drop for ComGuard { fn drop(&mut self) {
    if self.0 != 0 {
        // SAFETY: CoUninitialize() balances the successful CoInitializeEx of this thread.
        let f: unsafe extern "system" fn() = unsafe { core::mem::transmute(self.0) };
        unsafe { f() };
    }
} }
struct Handle(isize);
impl Drop for Handle { fn drop(&mut self) {
    // SAFETY: uniquely owned Windows handle.
    if self.0 != 0 && self.0 != -1 { unsafe { CloseHandle(self.0); } }
} }
fn error() -> u32 {
    // SAFETY: reads the current thread's last-error slot.
    unsafe { GetLastError() }
}
fn pipe(parent_reads: bool) -> Result<(Handle, Handle), u32> {
    let mut r = 0; let mut w = 0;
    let sa = Security { size: core::mem::size_of::<Security>() as u32, descriptor: 0, inherit: 1 };
    // SAFETY: valid output pointers and initialized SECURITY_ATTRIBUTES.
    if unsafe { CreatePipe(&mut r, &mut w, &sa, 65536) } == 0 { return Err(error()); }
    let pair = (Handle(r), Handle(w));
    // SAFETY: clear inheritance on the endpoint retained by the parent.
    if unsafe { SetHandleInformation(if parent_reads { r } else { w }, 1, 0) } == 0 { return Err(error()); }
    Ok(pair)
}
fn quote(s: &str) -> String {
    let mut out = String::from("\""); let mut slashes = 0;
    for ch in s.chars() {
        if ch == '\\' { slashes += 1; continue; }
        for _ in 0..if ch == '"' { slashes * 2 + 1 } else { slashes } { out.push('\\'); }
        slashes = 0; out.push(ch);
    }
    for _ in 0..slashes * 2 { out.push('\\'); } out.push('"'); out
}
fn fold(u: u16) -> u32 {
    char::from_u32(u as u32).map_or(u as u32, |c| c.to_lowercase().next().unwrap_or(c) as u32)
}
fn environment(c: &Command<'_>) -> Result<Vec<u16>, u32> {
    if c.env.is_empty() { return Ok(Vec::new()); }
    for (key, value) in c.env {
        if key.is_empty() || key.contains(['=', '\0']) || value.is_some_and(|v| v.contains('\0')) { return Err(87); }
    }
    // entries stay UTF-16 so unpaired surrogates of the inherited block are passed on unchanged
    let mut entries: Vec<Vec<u16>> = Vec::new();
    // SAFETY: OS supplies a double-NUL terminated environment block; freed after copying.
    unsafe {
        let p = GetEnvironmentStringsW(); if p.is_null() { return Err(error()); }
        let mut n = 0;
        while *p.add(n) != 0 {
            let start = n; while *p.add(n) != 0 { n += 1; }
            entries.push(core::slice::from_raw_parts(p.add(start), n - start).to_vec()); n += 1;
        }
        FreeEnvironmentStringsW(p);
    }
    for (key, value) in c.env {
        let k: Vec<u32> = key.encode_utf16().map(fold).collect();
        entries.retain(|s| {
            let at = s.iter().position(|&u| u == b'=' as u16).unwrap_or(s.len());
            !(at == k.len() && s[..at].iter().map(|&u| fold(u)).eq(k.iter().copied()))
        });
        if let Some(v) = value { entries.push(format!("{key}={v}").encode_utf16().collect()); }
    }
    entries.sort_by_key(|s| s.iter().map(|&u| fold(u)).collect::<Vec<u32>>());
    let mut block = Vec::new(); for s in entries { block.extend(s); block.push(0); }
    if block.is_empty() { block.push(0); } block.push(0); Ok(block)
}
/// Native child with captured output and a kill-on-close Windows job.
pub struct Child { process: Handle, job: Handle, input: Option<Handle>, output: Handle, error: Handle }
pub(super) fn spawn(c: &Command<'_>) -> Result<Child, u32> {
    let mut line = quote(c.program); for a in c.args { line.push(' '); line.push_str(&quote(a)); }
    let mut line: Vec<u16> = line.encode_utf16().chain(core::iter::once(0)).collect();
    if line.len() > 32767 { return Err(206); }
    let dir = c.cwd.map(crate::to_wide); let env = environment(c)?;
    let (ir, iw) = pipe(false)?; let (or, ow) = pipe(true)?; let (er, ew) = pipe(true)?;
    // SAFETY: anonymous job, valid zeroed extended-limit layout (144 bytes on x64).
    let job = Handle(unsafe { CreateJobObjectW(0, core::ptr::null()) });
    if job.0 == 0 { return Err(error()); }
    let mut limits = [0u64; 18]; limits[2] = 0x2000;
    // SAFETY: initialized JOBOBJECT_EXTENDED_LIMIT_INFORMATION with KILL_ON_JOB_CLOSE.
    if unsafe { SetInformationJobObject(job.0, 9, limits.as_ptr().cast(), 144) } == 0 { return Err(error()); }
    let mut size = 0usize;
    // SAFETY: size query, followed by aligned allocation for one handle-list attribute.
    unsafe { InitializeProcThreadAttributeList(0, 1, 0, &mut size); }
    let mut attrs = vec![0usize; size.div_ceil(core::mem::size_of::<usize>())];
    let list = attrs.as_mut_ptr() as usize;
    // SAFETY: buffer is aligned and sized by the OS query.
    if unsafe { InitializeProcThreadAttributeList(list, 1, 0, &mut size) } == 0 { return Err(error()); }
    let handles = [ir.0, ow.0, ew.0];
    let mut sx = StartupEx { info: StartupInfo { cb: 112, rest: [0; 25] }, attrs: list };
    sx.info.rest[14] = 0x100;
    for (index, h) in [(19, ir.0), (21, ow.0), (23, ew.0)] {
        sx.info.rest[index] = h as u32; sx.info.rest[index + 1] = (h as u64 >> 32) as u32;
    }
    let mut pi = [0isize; 3];
    // SAFETY: explicit inheritable endpoints and live UTF-16 strings; child starts suspended.
    let (ok, err) = unsafe {
        let attr_ok = UpdateProcThreadAttribute(list, 0, 0x20002, handles.as_ptr() as usize, 24, 0, 0);
        let ok = if attr_ok == 0 { 0 } else { CreateProcessW(core::ptr::null(), line.as_mut_ptr(), 0, 0, 1,
            0x08080004 | 0x400, if env.is_empty() { 0 } else { env.as_ptr() as usize },
            dir.as_ref().map_or(core::ptr::null(), |d| d.as_ptr()), &sx.info, &mut pi) };
        let err = GetLastError(); DeleteProcThreadAttributeList(list); (ok, err)
    };
    if ok == 0 { return Err(err); }
    let process = Handle(pi[0]); let thread = Handle(pi[1]);
    // SAFETY: child is suspended until attached to the owned job, preventing escaping children.
    unsafe {
        if AssignProcessToJobObject(job.0, process.0) == 0 {
            let err = error(); TerminateProcess(process.0, 1); return Err(err);
        }
        if ResumeThread(thread.0) == u32::MAX {
            let err = error(); TerminateJobObject(job.0, 1); return Err(err);
        }
    }
    Ok(Child { process, job, input: Some(iw), output: or, error: er })
}
fn read(h: &Handle, bytes: &mut [u8]) -> Result<Option<usize>, u32> {
    if bytes.is_empty() { return Ok(None); } // no room: neither data nor EOF
    let mut available = 0; let mut n = 0;
    // SAFETY: PeekNamedPipe queries available bytes; ReadFile reads at most that many.
    unsafe {
        if PeekNamedPipe(h.0, 0, 0, 0, &mut available, 0) == 0 {
            let e = error(); return if e == 109 { Ok(Some(0)) } else { Err(e) };
        }
        if available == 0 { return Ok(None); }
        if ReadFile(h.0, bytes.as_mut_ptr(), available.min(bytes.len().min(u32::MAX as usize) as u32), &mut n, core::ptr::null_mut()) == 0 { return Err(error()); }
    }
    Ok(Some(n as usize))
}
impl Child {
    /// Read stdout without waiting; None means no data, Some(0) means EOF.
    pub fn read_stdout(&mut self, bytes: &mut [u8]) -> Result<Option<usize>, u32> { read(&self.output, bytes) }
    /// Read stderr without waiting; None means no data, Some(0) means EOF.
    pub fn read_stderr(&mut self, bytes: &mut [u8]) -> Result<Option<usize>, u32> { read(&self.error, bytes) }
    /// Write a small chunk to stdin. Call from an IO worker, not the UI thread.
    pub fn write_stdin(&mut self, bytes: &[u8]) -> Result<Option<usize>, u32> {
        let h = self.input.as_ref().ok_or(109u32)?; let mut n = 0;
        // SAFETY: live input buffer, owned synchronous pipe, bounded write.
        if unsafe { WriteFile(h.0, bytes.as_ptr(), bytes.len().min(4096) as u32, &mut n, core::ptr::null_mut()) } == 0 { return Err(error()); }
        Ok(Some(n as usize))
    }
    /// Close stdin to request orderly helper shutdown.
    pub fn close_stdin(&mut self) { self.input = None; }
    /// Return the child exit status without blocking.
    pub fn try_wait(&mut self) -> Result<Option<i32>, u32> {
        // SAFETY: owned process handle and valid output pointer.
        unsafe {
            let r = WaitForSingleObject(self.process.0, 0);
            if r == 258 { return Ok(None); } if r == u32::MAX { return Err(error()); }
            let mut code = 0; if GetExitCodeProcess(self.process.0, &mut code) == 0 { Err(error()) } else { Ok(Some(code as i32)) }
        }
    }
    /// Terminate all processes in the owned job.
    pub fn terminate(&mut self) {
        // SAFETY: this job contains only this child and descendants.
        unsafe { TerminateJobObject(self.job.0, 1); WaitForSingleObject(self.process.0, 200); }
    }
}
impl Drop for Child { fn drop(&mut self) { self.terminate(); } }
