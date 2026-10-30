// win_autostart -- (autostart.rs) the Run key of the user on Windows
//
//   set(on, args)   on: HKCU\...\Run\<app id> = "exe" args (REG_SZ) and the Task Manager's
//                   "off" (Explorer\StartupApproved\Run\<app id>) removed; off: both removed
//   get()           the Run value is there and StartupApproved does not say off (odd first
//                   byte)
//   entry()         the Run value
//   In an MSIX package (a package identity, GetCurrentPackageFullName) HKCU is virtualized:
//   the Run value would reach no one.  There the package's StartupTask (TaskId "Autostart",
//   written by tools/pkg_msix.py) is switched through WinRT (win_winrt.rs, on an MTA thread):
//   StartupTask.GetAsync, then RequestEnableAsync (its new state: Enabled / EnabledByPolicy,
//   else Dismissed -- the user switched it off in Settings -> Apps -> Startup, or a policy
//   did) or Disable(); get() = State Enabled / EnabledByPolicy (kept for a second); entry() =
//   the program (a StartupTask takes no arguments: args are not passed); a manifest without
//   the task: Unavailable.
//   (crate) quote_arg(arg, &mut out)   one argument with the Microsoft rules (CommandLineToArgvW:
//                   quotes when needed, n backslashes before a quote doubled + 1); also the
//                   arguments of the taskbar's tasks (win_taskbar.rs)

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU64, Ordering};

use super::AutostartError;
use crate::win_winrt::{self, Guid, Obj};

#[link(name = "advapi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn RegSetKeyValueW(key: isize, sub: *const u16, name: *const u16, typ: u32, data: *const u8, n: u32) -> i32;
    fn RegGetValueW(key: isize, sub: *const u16, name: *const u16, flags: u32, typ: *mut u32, data: *mut u8, n: *mut u32) -> i32;
    fn RegDeleteKeyValueW(key: isize, sub: *const u16, name: *const u16) -> i32;
}

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetCurrentPackageFullName(len: *mut u32, name: *mut u16) -> i32;
}

const HKCU: isize = 0xFFFF_FFFF_8000_0001u64 as isize;
const RUN: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
const APPROVED: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\StartupApproved\\Run";
const REG_SZ: u32 = 1;
const RRF_RT_ANY: u32 = 0xFFFF;
const ERROR_FILE_NOT_FOUND: i32 = 2;
// the longest command line CreateProcess takes (wide chars)
const CMD_MAX: usize = 32767;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(core::iter::once(0)).collect()
}

/// a value of HKCU\sub as raw bytes (None: missing or unreadable)
fn reg_get(sub: &str, name: &str) -> Option<(u32, Vec<u8>)> {
    let (ws, wn) = (wide(sub), wide(name));
    let (mut typ, mut n) = (0u32, 0u32);
    // SAFETY: RegGetValueW with NUL-terminated names; first only the size, then into a buffer
    // of that size (retried once when the value grew meanwhile).
    unsafe {
        if RegGetValueW(HKCU, ws.as_ptr(), wn.as_ptr(), RRF_RT_ANY, &mut typ, core::ptr::null_mut(), &mut n) != 0 {
            return None;
        }
        for _ in 0..2 {
            let mut buf = alloc::vec![0u8; n as usize + 2];
            let mut m = buf.len() as u32;
            if RegGetValueW(HKCU, ws.as_ptr(), wn.as_ptr(), RRF_RT_ANY, &mut typ, buf.as_mut_ptr(), &mut m) == 0 {
                buf.truncate(m as usize);
                return Some((typ, buf));
            }
            n = m;
        }
    }
    None
}

fn reg_delete(sub: &str, name: &str) -> bool {
    let (ws, wn) = (wide(sub), wide(name));
    // SAFETY: NUL-terminated names.
    let r = unsafe { RegDeleteKeyValueW(HKCU, ws.as_ptr(), wn.as_ptr()) };
    r == 0 || r == ERROR_FILE_NOT_FOUND
}

/// append arg with the Microsoft quoting rules (CommandLineToArgvW reads it back)
pub(crate) fn quote_arg(arg: &str, out: &mut String) {
    if !arg.is_empty() && !arg.contains([' ', '\t', '\n', '\x0B', '"']) {
        out.push_str(arg);
        return;
    }
    out.push('"');
    let mut bs = 0; // backslashes seen
    for c in arg.chars() {
        match c {
            '\\' => bs += 1,
            '"' => {
                out.extend(core::iter::repeat_n('\\', bs + 1));
                bs = 0;
            }
            _ => bs = 0,
        }
        out.push(c);
    }
    out.extend(core::iter::repeat_n('\\', bs)); // before the closing quote
    out.push('"');
}

/// "exe" arg1 arg2 ...
pub(crate) fn command_line(exe: &str, args: &[&str]) -> String {
    let mut s = format!("\"{exe}\"");
    for a in args {
        s.push(' ');
        quote_arg(a, &mut s);
    }
    s
}

/// this process has a package identity (MSIX): ERROR_INSUFFICIENT_BUFFER for the length query,
/// APPMODEL_ERROR_NO_PACKAGE (15700) without one
pub(crate) fn packaged() -> bool {
    let mut n = 0u32;
    // SAFETY: a length query without a buffer; the call only writes the needed length into n.
    unsafe { GetCurrentPackageFullName(&mut n, core::ptr::null_mut()) == 122 }
}

pub fn set(on: bool, args: &[&str]) -> Result<(), AutostartError> {
    if packaged() {
        STATE_AT.store(0, Ordering::SeqCst);
        return crate::win_winrt::mta(move || task_set(on)).unwrap_or_else(|| Err(AutostartError::Failed(String::from("no thread for WinRT"))));
    }
    let name = crate::vfs::app_id();
    if !on {
        let ok = reg_delete(RUN, &name);
        reg_delete(APPROVED, &name);
        return if ok { Ok(()) } else { Err(AutostartError::Failed(String::from("RegDeleteKeyValueW failed"))) };
    }
    let exe = crate::current_exe().ok_or_else(|| AutostartError::Failed(String::from("no path of the program")))?;
    let cmd = wide(&command_line(&exe.replace('/', "\\"), args));
    if cmd.len() > CMD_MAX {
        return Err(AutostartError::Invalid);
    }
    let (ws, wn) = (wide(RUN), wide(&name));
    // SAFETY: NUL-terminated names and a REG_SZ of cmd.len() wide chars including the NUL.
    let r = unsafe { RegSetKeyValueW(HKCU, ws.as_ptr(), wn.as_ptr(), REG_SZ, cmd.as_ptr() as *const u8, (cmd.len() * 2) as u32) };
    if r != 0 {
        return Err(AutostartError::Failed(format!("RegSetKeyValueW: error {r}")));
    }
    reg_delete(APPROVED, &name); // the Task Manager's "off" would hide the new entry
    Ok(())
}

pub fn get() -> bool {
    if packaged() {
        // the answer of the last second (a settings page may ask every frame; each answer
        // costs a thread and a WinRT call): bit 0 the state, the rest the time in ms + 1
        let now = crate::time_us() / 1000 + 1;
        let at = STATE_AT.load(Ordering::SeqCst);
        if at != 0 && now.saturating_sub(at >> 1) < STATE_CACHE_MS {
            return at & 1 != 0;
        }
        let on = crate::win_winrt::mta(|| task().and_then(|t| t.get_i32(ST_STATE)).is_ok_and(|s| s == ENABLED || s == ENABLED_BY_POLICY))
            .unwrap_or(false);
        STATE_AT.store(now << 1 | on as u64, Ordering::SeqCst);
        return on;
    }
    let name = crate::vfs::app_id();
    if reg_get(RUN, &name).is_none() {
        return false;
    }
    // StartupApproved: 12 bytes, the first even = on (02, 06), odd = off (03, 07)
    !reg_get(APPROVED, &name).is_some_and(|(_, d)| d.first().is_some_and(|b| b & 1 != 0))
}

pub fn entry() -> Option<String> {
    if packaged() {
        // the task starts the package's program, without arguments
        let exists = crate::win_winrt::mta(|| task().is_ok()).unwrap_or(false);
        return exists.then(|| crate::current_exe().map(|e| command_line(&e.replace('/', "\\"), &[]))).flatten();
    }
    let (_, d) = reg_get(RUN, &crate::vfs::app_id())?;
    let w: Vec<u16> = d.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|&c| c != 0).collect();
    Some(String::from_utf16_lossy(&w))
}

// ---------------------------------------------------------------- the StartupTask (MSIX)

/// the TaskId tools/pkg_msix.py writes into the AppxManifest
const TASK_ID: &str = "Autostart";
static IID_ISTARTUPTASKSTATICS: Guid = Guid(0xEE5B_60BD, 0xA148, 0x41A7, [0xB2, 0x6E, 0xE8, 0xB8, 0x8A, 0x1E, 0x62, 0xF8]);
// IStartupTaskStatics.GetAsync; IStartupTask.RequestEnableAsync / Disable / State;
// IAsyncOperation<T>.GetResults
const SS_GETASYNC: usize = 7;
const ST_REQUESTENABLE: usize = 6;
const ST_DISABLE: usize = 7;
const ST_STATE: usize = 8;
const OP_RESULTS: usize = 8;
// StartupTaskState
const DISABLED_BY_USER: i32 = 1;
const ENABLED: i32 = 2;
const DISABLED_BY_POLICY: i32 = 3;
const ENABLED_BY_POLICY: i32 = 4;
// how long GetAsync / Disable may take (RequestEnableAsync: AUTOSTART_WAIT_MS)
const TASK_MS: u32 = 15_000;
// get(): the task's state is asked again after this (set() forgets it)
const STATE_CACHE_MS: u64 = 1000;
static STATE_AT: AtomicU64 = AtomicU64::new(0);
// the manifest has no such task: HRESULT_FROM_WIN32(ERROR_NOT_FOUND), E_INVALIDARG
const E_NOT_FOUND: i32 = 0x8007_0490u32 as i32;
const E_INVALIDARG: i32 = 0x8007_0057u32 as i32;

fn task_error(hr: i32) -> AutostartError {
    if hr == E_NOT_FOUND || hr == E_INVALIDARG {
        AutostartError::Unavailable
    } else {
        AutostartError::Failed(format!("StartupTask: 0x{:08X}", hr as u32))
    }
}

/// StartupTask.GetAsync("Autostart") (on an MTA thread: win_winrt::mta)
fn task() -> Result<Obj, i32> {
    let statics = win_winrt::factory("Windows.ApplicationModel.StartupTask", &IID_ISTARTUPTASKSTATICS)?;
    let id = win_winrt::HStr::new(TASK_ID).ok_or(win_winrt::E_FAIL)?;
    let mut op = 0usize;
    // SAFETY: GetAsync(this, HSTRING taskId, out IAsyncOperation<StartupTask>).
    let hr = unsafe {
        let f: unsafe extern "system" fn(usize, isize, *mut usize) -> i32 = core::mem::transmute(statics.vfn(SS_GETASYNC));
        f(statics.ptr(), id.raw(), &mut op)
    };
    if hr < 0 {
        return Err(hr);
    }
    let op = Obj::own(op).ok_or(win_winrt::E_FAIL)?;
    win_winrt::wait(&op, TASK_MS)?;
    op.get_obj(OP_RESULTS)
}

/// switch the task (on an MTA thread)
fn task_set(on: bool) -> Result<(), AutostartError> {
    let t = task().map_err(task_error)?;
    if !on {
        return t.call0(ST_DISABLE).map_err(task_error);
    }
    match t.get_i32(ST_STATE).map_err(task_error)? {
        ENABLED | ENABLED_BY_POLICY => return Ok(()),
        DISABLED_BY_USER | DISABLED_BY_POLICY => return Err(AutostartError::Dismissed),
        _ => {}
    }
    let op = t.get_obj(ST_REQUESTENABLE).map_err(task_error)?;
    win_winrt::wait(&op, super::AUTOSTART_WAIT_MS as u32).map_err(|hr| if hr == win_winrt::E_TIMEOUT { AutostartError::Dismissed } else { task_error(hr) })?;
    match op.get_i32(OP_RESULTS).map_err(task_error)? {
        ENABLED | ENABLED_BY_POLICY => Ok(()),
        _ => Err(AutostartError::Dismissed),
    }
}
