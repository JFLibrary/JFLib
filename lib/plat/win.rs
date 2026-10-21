// win -- Windows platform: entry, time, arguments, UTF-16 (files, environment: win_sys.rs;
// threads, wake-up: win_thread.rs; the user32/gdi32 windows: win_api.rs)
//                                                                  [src/plat/win/plat_win.s]

use alloc::string::String;
use alloc::vec::Vec;

#[path = "win_api.rs"]
mod api;
pub use api::*;
#[path = "win_sys.rs"]
mod sys;
pub use sys::*;
#[path = "win_thread.rs"]
mod thread;
pub use thread::*;
#[path = "win_crash.rs"]
mod crash;
// plat-shared: printing, plugin libraries + crash guard
#[path = "win_print.rs"]
mod wprint;
pub use wprint::*;
#[path = "win_lib.rs"]
mod wlib;
pub use wlib::*;

#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetCommandLineW() -> *const u16;
    fn QueryPerformanceCounter(c: *mut i64) -> i32;
    fn QueryPerformanceFrequency(f: *mut i64) -> i32;
    fn Sleep(ms: u32);
}

unsafe extern "Rust" {
    fn jfgui_main() -> i32;
}

// PE entry point (lld-link /entry:mainCRTStartup): the app's jfgui_main, then exit
#[unsafe(no_mangle)]
pub extern "system" fn mainCRTStartup() -> ! {
    crash::crash_init();
    thread::wake_init();
    crate::cpu::simd_override();
    rt::boot::set_handler(crate::boot::start); // rt::main!: name, app://, manifest
    // SAFETY: the app defines jfgui_main (ui::main! macro).
    let code = unsafe { jfgui_main() };
    rt::os::exit(code)
}

pub fn time_us() -> u64 {
    let (mut c, mut f) = (0i64, 0i64);
    // SAFETY: plain out-parameters.
    unsafe {
        QueryPerformanceCounter(&mut c);
        QueryPerformanceFrequency(&mut f);
    }
    (c as u128 * 1_000_000 / f.max(1) as u128) as u64
}

pub fn sleep_ms(ms: u32) {
    // SAFETY: no memory involved.
    unsafe { Sleep(ms) }
}

// ---- UTF-16 conversion
pub fn to_wide(s: &str) -> Vec<u16> {
    let mut w: Vec<u16> = s.encode_utf16().collect();
    w.push(0);
    w
}

pub fn from_wide(w: &[u16]) -> String {
    let mut s = String::new();
    for c in core::char::decode_utf16(w.iter().copied()) {
        s.push(c.unwrap_or('\u{FFFD}'));
    }
    s
}

// command line -> arguments (CommandLineToArgvW rules); argument 0 is the program
pub fn args() -> Vec<String> {
    // SAFETY: GetCommandLineW returns a NUL-terminated string that lives for the process.
    let cl = unsafe {
        let p = GetCommandLineW();
        let mut n = 0;
        while *p.add(n) != 0 {
            n += 1;
        }
        core::slice::from_raw_parts(p, n)
    };
    let mut out = Vec::new();
    let mut i = 0;
    let n = cl.len();
    // program name: up to the next blank, quotes taken literally
    let mut cur: Vec<u16> = Vec::new();
    let mut quoted = false;
    while i < n {
        let c = cl[i];
        if c == b'"' as u16 {
            quoted = !quoted;
        } else if !quoted && (c == b' ' as u16 || c == b'\t' as u16) {
            break;
        } else {
            cur.push(c);
        }
        i += 1;
    }
    out.push(from_wide(&cur));
    loop {
        while i < n && (cl[i] == b' ' as u16 || cl[i] == b'\t' as u16) {
            i += 1;
        }
        if i >= n {
            break;
        }
        cur.clear();
        let mut q = false;
        while i < n {
            let c = cl[i];
            if !q && (c == b' ' as u16 || c == b'\t' as u16) {
                break;
            }
            if c == b'\\' as u16 {
                let mut k = 0;
                while i < n && cl[i] == b'\\' as u16 {
                    k += 1;
                    i += 1;
                }
                if i < n && cl[i] == b'"' as u16 {
                    for _ in 0..k / 2 {
                        cur.push(b'\\' as u16);
                    }
                    if k % 2 == 1 {
                        cur.push(b'"' as u16);
                        i += 1;
                    }
                } else {
                    for _ in 0..k {
                        cur.push(b'\\' as u16);
                    }
                }
                continue;
            }
            if c == b'"' as u16 {
                if q && i + 1 < n && cl[i + 1] == b'"' as u16 {
                    cur.push(b'"' as u16);
                    i += 2;
                    continue;
                }
                q = !q;
                i += 1;
                continue;
            }
            cur.push(c);
            i += 1;
        }
        out.push(from_wide(&cur));
    }
    out
}
