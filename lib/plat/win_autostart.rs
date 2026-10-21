// win_autostart -- (autostart.rs) the Run key of the user on Windows
//
//   set(on, args)   on: HKCU\...\Run\<app id> = "exe" args (REG_SZ) and the Task Manager's
//                   "off" (Explorer\StartupApproved\Run\<app id>) removed; off: both removed
//   get()           the Run value is there and StartupApproved does not say off (odd first
//                   byte)
//   entry()         the Run value
//   In an MSIX package (a package identity, GetCurrentPackageFullName) HKCU is virtualized:
//   the Run value would reach no one.  There set() is Unavailable, get() false, entry() None;
//   the package's StartupTask (tools/pkg_msix.py) is switched on in Settings -> Apps -> Startup.
//   (crate) quote_arg(arg, &mut out)   one argument with the Microsoft rules (CommandLineToArgvW:
//                   quotes when needed, n backslashes before a quote doubled + 1); also the
//                   arguments of the taskbar's tasks (win_taskbar.rs)

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use super::AutostartError;

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
        return Err(AutostartError::Unavailable);
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
        return false;
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
        return None;
    }
    let (_, d) = reg_get(RUN, &crate::vfs::app_id())?;
    let w: Vec<u16> = d.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|&c| c != 0).collect();
    Some(String::from_utf16_lossy(&w))
}
