// linux -- Linux platform: raw syscalls, no libc        [src/plat/linux/plat_linux.s, lx_sys.s]
//
// _start reads argc / argv / envp from the initial stack, calls the app's jfgui_main and
// exits with its result.  Files and paths: linux_sys.rs; threads, wake-up, system
// information: linux_thread.rs; windows: linux_disp.rs (Wayland / X11 backends).

use alloc::string::String;
use alloc::vec::Vec;

#[path = "linux_disp.rs"]
mod disp;
pub use disp::*;
#[path = "linux_wl.rs"]
mod wl;
pub use wl::*;
#[path = "linux_x11.rs"]
mod x11;
pub use x11::*;
#[path = "linux_portal.rs"]
mod portal;
pub use portal::*;
// tray.rs: the symbol in the notification area and the notifications (D-Bus)
#[path = "linux_tray.rs"]
pub(crate) mod linux_tray;
// a small session-bus connection of its own (linux_tray.rs, linux_ibus.rs)
#[path = "linux_dbus.rs"]
pub(crate) mod linux_dbus;
// the documents portal: lasting document IDs (persistent_name)
#[path = "linux_docs.rs"]
pub(crate) mod linux_docs;
// a11y.rs: the accessibility tree on the AT-SPI2 bus (D-Bus)
#[path = "linux_atspi.rs"]
pub(crate) mod linux_atspi;
// input methods on X11: IBus through the session bus portal
#[path = "linux_ibus.rs"]
mod linux_ibus;
// lifecycle: SIGTERM, logind's sleep, the settings portal
#[path = "linux_life.rs"]
mod linux_life;
// the system settings from the settings portal (settings.rs)
#[path = "linux_settings.rs"]
pub(crate) mod linux_settings;
pub(crate) use linux_settings::sys_settings_os;
pub use linux_life::{life_poll, life_start};
pub use linux_ibus::{IBUS_RELEASE, ibus_available, ibus_focus, ibus_key, ibus_ready};
#[path = "linux_sys.rs"]
mod sys;
pub use sys::*;
#[path = "linux_thread.rs"]
mod thread;
pub use thread::*;
#[path = "linux_crash.rs"]
mod crash;
// plat-shared: printing, plugin libraries (own ELF loader) + crash guard
#[path = "linux_print.rs"]
mod lprint;
pub use lprint::*;
#[path = "linux_elfload.rs"]
mod elfload;
pub use elfload::*;
#[path = "linux_guard.rs"]
mod guard;
pub(crate) use guard::*;

use rt::os::*;

// the initial stack: argc, argv[], 0, envp[], 0
static mut START_SP: *const usize = core::ptr::null();

unsafe extern "Rust" {
    fn jfgui_main() -> i32;
}

core::arch::global_asm!(
    ".globl _start",
    "_start:",
    "    xor ebp, ebp",
    "    mov rdi, rsp",
    "    and rsp, -16",
    "    call {entry}",
    "    ud2",
    entry = sym linux_entry,
);

extern "C" fn linux_entry(sp: *const usize) -> ! {
    // SAFETY: written once before anything reads it, on the only thread.
    unsafe { START_SP = sp };
    crash::crash_init();
    thread::wake_init();
    crate::cpu::simd_override();
    rt::boot::set_handler(crate::boot::start); // rt::main!: name, app://, manifest
    let code = unsafe { jfgui_main() };
    rt::os::exit(code)
}

pub fn time_us() -> u64 {
    let mut ts = [0i64; 2];
    // SAFETY: CLOCK_MONOTONIC into a timespec.
    unsafe { syscall3(SYS_CLOCK_GETTIME, 1, ts.as_mut_ptr() as usize, 0) };
    ts[0] as u64 * 1_000_000 + ts[1] as u64 / 1000
}

/// wall-clock time: nanoseconds since 1970-01-01 00:00 UTC (CLOCK_REALTIME)
pub fn unix_time_ns() -> i64 {
    let mut ts = [0i64; 2];
    // SAFETY: CLOCK_REALTIME into a timespec.
    unsafe { syscall3(SYS_CLOCK_GETTIME, 0, ts.as_mut_ptr() as usize, 0) };
    ts[0].wrapping_mul(1_000_000_000).wrapping_add(ts[1])
}

// C string at p -> String
unsafe fn cstr(p: *const u8) -> String {
    let mut n = 0;
    while unsafe { *p.add(n) } != 0 {
        n += 1;
    }
    String::from_utf8_lossy(unsafe { core::slice::from_raw_parts(p, n) }).into_owned()
}

pub fn args() -> Vec<String> {
    let mut v = Vec::new();
    // SAFETY: START_SP points to argc and the argv array the kernel built.
    unsafe {
        let sp = START_SP;
        if sp.is_null() {
            return v;
        }
        let argc = *sp;
        for i in 0..argc {
            v.push(cstr(*sp.add(1 + i) as *const u8));
        }
    }
    v
}

// ---- environment: the initial block, copied on the first access so that setenv can
// replace entries (readers on other threads see either list, both stay valid)
static ENV: rt::sync::SpinLock<Option<Vec<Vec<u8>>>> = rt::sync::SpinLock::new(None);

pub(crate) fn process_environment() -> Vec<Vec<u8>> {
    env_list(&mut ENV.lock()).clone()
}

fn env_list(g: &mut Option<Vec<Vec<u8>>>) -> &mut Vec<Vec<u8>> {
    g.get_or_insert_with(|| {
        let mut v = Vec::new();
        // SAFETY: envp follows argv and its terminating 0 on the initial stack.
        unsafe {
            let sp = START_SP;
            if !sp.is_null() {
                let argc = *sp;
                let mut e = sp.add(argc + 2);
                while *e != 0 {
                    let p = *e as *const u8;
                    let mut n = 0;
                    while *p.add(n) != 0 {
                        n += 1;
                    }
                    v.push(core::slice::from_raw_parts(p, n).to_vec());
                    e = e.add(1);
                }
            }
        }
        v
    })
}

/// the value of an environment variable  [plat_env / plat_getenv]
pub fn env(name: &str) -> Option<String> {
    let mut g = ENV.lock();
    let n = name.as_bytes();
    env_list(&mut g)
        .iter()
        .find(|e| e.len() > n.len() && e.starts_with(n) && e[n.len()] == b'=')
        .map(|e| String::from_utf8_lossy(&e[n.len() + 1..]).into_owned())
}

/// the same as env (ASMLIB's plat_getenv)
pub fn getenv(name: &str) -> Option<String> {
    env(name)
}

/// set (Some) or remove (None) an environment variable of this process  [plat_setenv]
pub fn setenv(name: &str, value: Option<&str>) -> bool {
    let mut g = ENV.lock();
    let list = env_list(&mut g);
    let n = name.as_bytes();
    list.retain(|e| !(e.len() > n.len() && e.starts_with(n) && e[n.len()] == b'='));
    if let Some(v) = value {
        let mut e = n.to_vec();
        e.push(b'=');
        e.extend_from_slice(v.as_bytes());
        list.push(e);
    }
    true
}

pub fn sleep_ms(ms: u32) {
    // SAFETY: poll with no descriptors = a sleep.
    unsafe { syscall3(SYS_POLL, 0, 0, ms as usize) };
}
