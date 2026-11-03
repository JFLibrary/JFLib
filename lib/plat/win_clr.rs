// win_clr -- run a managed program on the CoreCLR another application ships
//
//   clr_run(dir, app, args)        self-contained .NET applications bring coreclr.dll and their
//                                  whole framework: load that runtime into this process, trust
//                                  every assembly of dir, execute app's Main -> its exit code.
//                                  No installed .NET is involved.  Plugin policy applies.
//   machine_reg_string(sub, name)  a string value of HKLM\sub (None: missing or another type)

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::ffi::{c_char, c_void};

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn LoadLibraryExW(path: *const u16, file: isize, flags: u32) -> isize;
    fn GetProcAddress(module: isize, name: *const u8) -> usize;
    fn GetLastError() -> u32;
}

#[link(name = "advapi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn RegGetValueW(key: isize, sub: *const u16, name: *const u16, flags: u32, typ: *mut u32, data: *mut u8, n: *mut u32) -> i32;
}

const HKLM: isize = 0xFFFF_FFFF_8000_0002u64 as isize;
const RRF_RT_REG_SZ: u32 = 2;
// LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_DEFAULT_DIRS: coreclr.dll's own folder
const LOAD_FLAGS: u32 = 0x1100;

// the hosting API of coreclr.dll (coreclrhost.h)
type Initialize = unsafe extern "C" fn(
    exe: *const c_char,
    domain: *const c_char,
    count: i32,
    keys: *const *const c_char,
    values: *const *const c_char,
    host: *mut *mut c_void,
    domain_id: *mut u32,
) -> i32;
type Execute = unsafe extern "C" fn(
    host: *mut c_void,
    domain_id: u32,
    argc: i32,
    argv: *const *const c_char,
    assembly: *const c_char,
    exit_code: *mut u32,
) -> i32;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(core::iter::once(0)).collect()
}

fn utf8z(s: &str) -> Vec<u8> {
    let mut v = Vec::from(s.as_bytes());
    v.push(0);
    v
}

fn pointers(strings: &[Vec<u8>]) -> Vec<*const c_char> {
    strings.iter().map(|s| s.as_ptr() as *const c_char).collect()
}

// the runtime refuses relative paths and paths with forward slashes
fn absolute(path: &str) -> String {
    let path = path.replace('/', "\\");
    if path.starts_with("\\\\") || path.as_bytes().get(1) == Some(&b':') {
        return path;
    }
    format!("{}\\{path}", crate::cwd().replace('/', "\\").trim_end_matches('\\'))
}

/// the runtime of dir runs app with args (no program name) on this thread
pub fn clr_run(dir: &str, app: &str, args: &[String]) -> Result<i32, String> {
    let (dir, app) = (absolute(dir), absolute(app));
    let runtime = format!("{}\\coreclr.dll", dir.trim_end_matches('\\'));
    if !crate::policy::plugin_allowed(&runtime) {
        return Err(format!("{runtime}: durch eine Richtlinie gesperrt"));
    }
    let app_dir = app.rsplit_once('\\').map_or(".", |v| v.0);
    // the trusted list takes absolute paths with backslashes only
    let mut trusted = String::new();
    crate::dir_list_raw(&dir, &mut |name, is_dir, _| {
        if !is_dir && name.len() > 4 && name[name.len() - 4..].eq_ignore_ascii_case(".dll") {
            trusted.push_str(dir.trim_end_matches('\\'));
            trusted.push('\\');
            trusted.push_str(name);
            trusted.push(';');
        }
    });
    let exe = absolute(&crate::current_exe().unwrap_or_default());
    let keys = [
        utf8z("TRUSTED_PLATFORM_ASSEMBLIES"),
        utf8z("NATIVE_DLL_SEARCH_DIRECTORIES"),
        utf8z("APP_PATHS"),
        utf8z("APP_CONTEXT_BASE_DIRECTORY"),
        utf8z("System.Runtime.Serialization.EnableUnsafeBinaryFormatterSerialization"),
    ];
    let values = [
        utf8z(&trusted),
        utf8z(&format!("{};", dir.trim_end_matches('\\'))),
        utf8z(app_dir),
        utf8z(&format!("{app_dir}\\")),
        utf8z("true"),
    ];
    let (key_ptrs, value_ptrs) = (pointers(&keys), pointers(&values));
    let argv: Vec<Vec<u8>> = args.iter().map(|a| utf8z(a)).collect();
    let argv_ptrs = pointers(&argv);
    let (exe, domain, app_z) = (utf8z(&exe), utf8z("PaintCraft.PdnHost"), utf8z(&app));
    // SAFETY: NUL-terminated strings that outlive the calls; the two entry points have the
    // signatures of coreclrhost.h; the handle stays loaded for the rest of the process.
    unsafe {
        let module = LoadLibraryExW(wide(&runtime).as_ptr(), 0, LOAD_FLAGS);
        if module == 0 {
            return Err(format!("{runtime} ließ sich nicht laden (Fehler {})", GetLastError()));
        }
        let (init, exec) = (GetProcAddress(module, c"coreclr_initialize".as_ptr() as *const u8), GetProcAddress(module, c"coreclr_execute_assembly".as_ptr() as *const u8));
        if init == 0 || exec == 0 {
            return Err(format!("{runtime} ist keine .NET-Laufzeit"));
        }
        let init: Initialize = core::mem::transmute(init);
        let exec: Execute = core::mem::transmute(exec);
        let (mut host, mut domain_id) = (core::ptr::null_mut(), 0u32);
        let hr = init(
            exe.as_ptr() as *const c_char,
            domain.as_ptr() as *const c_char,
            key_ptrs.len() as i32,
            key_ptrs.as_ptr(),
            value_ptrs.as_ptr(),
            &mut host,
            &mut domain_id,
        );
        if hr < 0 {
            return Err(format!("Die .NET-Laufzeit aus {dir} startet nicht (0x{:08X})", hr as u32));
        }
        let mut code = 0u32;
        let hr = exec(host, domain_id, argv_ptrs.len() as i32, argv_ptrs.as_ptr(), app_z.as_ptr() as *const c_char, &mut code);
        if hr < 0 {
            return Err(format!("{app} ließ sich nicht ausführen (0x{:08X})", hr as u32));
        }
        Ok(code as i32)
    }
}

/// a string value of HKLM\sub
pub fn machine_reg_string(sub: &str, name: &str) -> Option<String> {
    let (ws, wn) = (wide(sub), wide(name));
    let mut n = 0u32;
    // SAFETY: NUL-terminated names; first only the size, then a buffer of that size.
    unsafe {
        if RegGetValueW(HKLM, ws.as_ptr(), wn.as_ptr(), RRF_RT_REG_SZ, core::ptr::null_mut(), core::ptr::null_mut(), &mut n) != 0 {
            return None;
        }
        let mut buf = alloc::vec![0u16; n as usize / 2 + 1];
        let mut m = (buf.len() * 2) as u32;
        if RegGetValueW(HKLM, ws.as_ptr(), wn.as_ptr(), RRF_RT_REG_SZ, core::ptr::null_mut(), buf.as_mut_ptr() as *mut u8, &mut m) != 0 {
            return None;
        }
        let len = buf.iter().position(|c| *c == 0).unwrap_or(buf.len());
        Some(String::from_utf16_lossy(&buf[..len]))
    }
}
