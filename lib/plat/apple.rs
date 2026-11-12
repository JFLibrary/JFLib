// apple -- macOS and iOS through libSystem (docs/APPLE.md)
//
// Apple keeps only libSystem stable, not the system calls under it (decision 2), so everything here
// is a libSystem call, as on Android through Bionic.  This is the start of the port (AP2): process,
// environment, time, files (apple_sys.rs), threads, semaphores, the wake-up, system information
// (apple_thread.rs), crash reports (apple_crash.rs); windows answer as on a system without a display
// (apple_disp.rs) until AP4.
//
// main(): libSystem's start calls it (a program without std); it calls the app's jfgui_main and ends
// the process with its result.  The arguments and the environment come from _NSGetArgv and
// _NSGetEnviron, so a program with std (whose main is std's) reads the same.

use alloc::string::String;
use alloc::vec::Vec;

#[path = "apple_sys.rs"]
mod sys;
pub use sys::*;
#[path = "apple_thread.rs"]
mod thread;
pub use thread::*;
#[path = "apple_disp.rs"]
mod disp;
pub use disp::*;
// the Objective-C runtime (classes, selectors, typed objc_msgSend)
#[path = "apple_objc.rs"]
#[cfg_attr(target_os = "ios", allow(dead_code))] // (iOS: what UIKit's host needs of it)
pub(crate) mod objc;
// Objective-C blocks: global ones the system calls back, calling the system's (notifications)
#[path = "apple_block.rs"]
pub(crate) mod block;
// iOS's host: UIKit (AP5)
#[cfg(target_os = "ios")]
#[path = "apple_ios.rs"]
pub(crate) mod ios;
// (tests) VoiceOver's view of the elements (apple_ios_a11y.rs)
#[cfg(target_os = "ios")]
pub use ios::{IosAxAct, IosAxElem, IosAxText, ios_ax_act, ios_ax_escape, ios_ax_events, ios_ax_listen, ios_ax_select, ios_ax_text, ios_ax_tree, ios_ax_type};
// macOS's windows: AppKit (AP4)
#[cfg(target_os = "macos")]
#[path = "apple_mac.rs"]
pub(crate) mod mac;
#[cfg(target_os = "macos")]
pub use mac::*;
#[path = "apple_crash.rs"]
mod crash;

/// libSystem's functions and records this backend uses (Darwin's values, arm64)
#[allow(non_camel_case_types, dead_code)]
pub(crate) mod ffi {
    use core::ffi::{c_char, c_int, c_long, c_uint, c_void};

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    pub struct Timespec {
        pub tv_sec: i64,
        pub tv_nsec: i64,
    }

    /// struct stat with 64-bit inodes (the only one arm64 has)
    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    pub struct Stat {
        pub st_dev: i32,
        pub st_mode: u16,
        pub st_nlink: u16,
        pub st_ino: u64,
        pub st_uid: u32,
        pub st_gid: u32,
        pub st_rdev: i32,
        pub st_atimespec: Timespec,
        pub st_mtimespec: Timespec,
        pub st_ctimespec: Timespec,
        pub st_birthtimespec: Timespec,
        pub st_size: i64,
        pub st_blocks: i64,
        pub st_blksize: i32,
        pub st_flags: u32,
        pub st_gen: u32,
        pub st_lspare: i32,
        pub st_qspare: [i64; 2],
    }
    const _: () = assert!(core::mem::size_of::<Stat>() == 144);

    /// struct dirent with 64-bit inodes: d_ino, d_seekoff, d_reclen, d_namlen, d_type, d_name
    #[repr(C)]
    pub struct Dirent {
        pub d_ino: u64,
        pub d_seekoff: u64,
        pub d_reclen: u16,
        pub d_namlen: u16,
        pub d_type: u8,
        pub d_name: [u8; 1024],
    }

    #[repr(C)]
    pub struct Pollfd {
        pub fd: c_int,
        pub events: i16,
        pub revents: i16,
    }

    /// pthread_attr_t (64 bytes on arm64)
    #[repr(C, align(8))]
    pub struct PthreadAttr(pub [u8; 64]);

    // <fcntl.h>, <errno.h>, <sys/stat.h>, <time.h>, <poll.h>, <unistd.h>
    pub const O_RDONLY: c_int = 0x0000;
    pub const O_WRONLY: c_int = 0x0001;
    pub const O_RDWR: c_int = 0x0002;
    pub const O_NONBLOCK: c_int = 0x0004;
    pub const O_NOFOLLOW: c_int = 0x0100;
    pub const O_CREAT: c_int = 0x0200;
    pub const O_TRUNC: c_int = 0x0400;
    pub const O_EXCL: c_int = 0x0800;
    pub const O_DIRECTORY: c_int = 0x0010_0000;
    pub const O_CLOEXEC: c_int = 0x0100_0000;
    pub const F_SETFL: c_int = 4;
    pub const F_FULLFSYNC: c_int = 51;
    pub const LOCK_EX: c_int = 2;
    pub const LOCK_NB: c_int = 4;
    pub const EPERM: c_int = 1;
    pub const ENOENT: c_int = 2;
    pub const EINTR: c_int = 4;
    pub const EEXIST: c_int = 17;
    pub const ENOTDIR: c_int = 20;
    pub const EISDIR: c_int = 21;
    pub const EINVAL: c_int = 22;
    pub const EAGAIN: c_int = 35;
    pub const ENOTEMPTY: c_int = 66;
    pub const S_IFMT: u16 = 0o170000;
    pub const S_IFDIR: u16 = 0o040000;
    pub const S_IFREG: u16 = 0o100000;
    pub const DT_DIR: u8 = 4;
    pub const CLOCK_REALTIME: c_uint = 0;
    pub const CLOCK_MONOTONIC: c_uint = 6;
    pub const POLLIN: i16 = 0x0001;
    pub const SC_NPROCESSORS_ONLN: c_int = 58;
    pub const DISPATCH_TIME_NOW: u64 = 0;
    pub const DISPATCH_TIME_FOREVER: u64 = !0;

    #[link(name = "System")]
    unsafe extern "C" {
        // open, openat and fcntl are variadic: on Apple's arm64 the variadic arguments go on the
        // stack, so they must be declared as such (docs/APPLE.md, Fallen)
        pub fn open(path: *const c_char, flags: c_int, ...) -> c_int;
        pub fn fcntl(fd: c_int, cmd: c_int, ...) -> c_int;
        pub fn close(fd: c_int) -> c_int;
        pub fn read(fd: c_int, buf: *mut c_void, n: usize) -> isize;
        pub fn write(fd: c_int, buf: *const c_void, n: usize) -> isize;
        pub fn pread(fd: c_int, buf: *mut c_void, n: usize, off: i64) -> isize;
        pub fn pwrite(fd: c_int, buf: *const c_void, n: usize, off: i64) -> isize;
        pub fn fstat(fd: c_int, st: *mut Stat) -> c_int;
        pub fn stat(path: *const c_char, st: *mut Stat) -> c_int;
        pub fn lstat(path: *const c_char, st: *mut Stat) -> c_int;
        pub fn mkdir(path: *const c_char, mode: u16) -> c_int;
        pub fn unlink(path: *const c_char) -> c_int;
        pub fn rmdir(path: *const c_char) -> c_int;
        pub fn rename(old: *const c_char, new: *const c_char) -> c_int;
        pub fn readlink(path: *const c_char, buf: *mut c_char, size: usize) -> isize;
        pub fn getcwd(buf: *mut c_char, size: usize) -> *mut c_char;
        pub fn getpid() -> c_int;
        pub fn fsync(fd: c_int) -> c_int;
        pub fn ftruncate(fd: c_int, len: i64) -> c_int;
        pub fn fchown(fd: c_int, uid: u32, gid: u32) -> c_int;
        pub fn fchmod(fd: c_int, mode: u16) -> c_int;
        pub fn flock(fd: c_int, op: c_int) -> c_int;
        pub fn opendir(path: *const c_char) -> *mut c_void;
        pub fn readdir(dir: *mut c_void) -> *mut Dirent;
        pub fn closedir(dir: *mut c_void) -> c_int;
        pub fn __error() -> *mut c_int;
        pub fn clock_gettime(clock: c_uint, ts: *mut Timespec) -> c_int;
        pub fn nanosleep(req: *const Timespec, rem: *mut Timespec) -> c_int;
        pub fn pipe(fds: *mut c_int) -> c_int;
        pub fn poll(fds: *mut Pollfd, n: c_uint, timeout_ms: c_int) -> c_int;
        pub fn sysconf(name: c_int) -> c_long;
        pub fn sysctlbyname(name: *const c_char, old: *mut c_void, oldlen: *mut usize, new: *mut c_void, newlen: usize) -> c_int;
        pub fn pthread_attr_init(attr: *mut PthreadAttr) -> c_int;
        pub fn pthread_attr_destroy(attr: *mut PthreadAttr) -> c_int;
        pub fn pthread_attr_setstacksize(attr: *mut PthreadAttr, size: usize) -> c_int;
        pub fn pthread_create(t: *mut usize, attr: *const PthreadAttr, start: extern "C" fn(*mut c_void) -> *mut c_void, arg: *mut c_void) -> c_int;
        pub fn pthread_join(t: usize, ret: *mut *mut c_void) -> c_int;
        pub fn pthread_detach(t: usize) -> c_int;
        pub fn signal(sig: c_int, handler: usize) -> usize;
        pub fn pthread_threadid_np(t: usize, id: *mut u64) -> c_int;
        pub fn dispatch_semaphore_create(value: isize) -> *mut c_void;
        pub fn dispatch_semaphore_signal(sem: *mut c_void) -> isize;
        pub fn dispatch_semaphore_wait(sem: *mut c_void, timeout: u64) -> isize;
        pub fn dispatch_time(when: u64, delta_ns: i64) -> u64;
        pub fn dispatch_release(object: *mut c_void);
        pub fn chdir(path: *const c_char) -> c_int;
        pub fn _NSGetArgc() -> *mut c_int;
        pub fn _NSGetArgv() -> *mut *mut *mut c_char;
        pub fn _NSGetEnviron() -> *mut *mut *mut c_char;
        pub fn _NSGetExecutablePath(buf: *mut c_char, size: *mut u32) -> c_int;
    }

    /// errno of the calling thread
    pub fn errno() -> c_int {
        // SAFETY: __error returns the calling thread's errno slot, always valid.
        unsafe { *__error() }
    }
}

unsafe extern "Rust" {
    fn jfgui_main() -> i32;
}

/// the entry libSystem's start calls (a program without std; std brings its own main)
#[cfg(not(feature = "std"))]
#[unsafe(no_mangle)]
pub extern "C" fn main(_argc: i32, _argv: *const *const u8) -> i32 {
    // SIGPIPE ignored, as plat's start does it on Linux: a write to a closed pipe is an error, not the
    // end of the process (sockets have SO_NOSIGPIPE as well); child processes get it back at default
    // SAFETY: a constant handler (SIG_IGN = 1) for SIGPIPE (13).
    unsafe { ffi::signal(13, 1) };
    crash::crash_init();
    thread::wake_init();
    crate::cpu::simd_override();
    // iOS: nothing starts a program in the simulator in a folder (simctl spawn has no shell): jfgui_CWD
    // names it (tools/apple.py sim_run: the repository, as for every other target's tests)
    #[cfg(target_os = "ios")]
    if let Some(dir) = env("jfgui_CWD").filter(|d| !d.is_empty() && !d.contains('\0')) {
        let mut z = dir.into_bytes();
        z.push(0);
        // SAFETY: a NUL-terminated path.
        unsafe { ffi::chdir(z.as_ptr().cast()) };
    }
    rt::boot::set_handler(crate::boot::start); // rt::main!: name, app://, manifest
    // iOS: the canvas the app's windows start in; the host's view sends its real size
    #[cfg(target_os = "ios")]
    ios::before_main();
    // SAFETY: the app defines jfgui_main (rt::main! / ui::main!); it takes no arguments.
    let code = unsafe { jfgui_main() };
    // iOS: an app installed its runtime (ui::launch): UIKit takes the main thread, never returns
    #[cfg(target_os = "ios")]
    if code == 0 && crate::web::installed() {
        ios::run();
    }
    rt::os::exit(code)
}

/// (test aid) the process's CPU time (us), resident memory and its own footprint (bytes), as on
/// Windows and Linux (render3dtest and uigputest measure with it)
pub fn win_test_usage() -> (u64, u64, u64) {
    unsafe extern "C" {
        fn getrusage(who: i32, usage: *mut u64) -> i32;
        static mach_task_self_: u32;
        fn task_info(task: u32, flavor: u32, info: *mut u64, count: *mut u32) -> i32;
    }
    let mut ru = [0u64; 18]; // struct rusage: two timevals (16 bytes each) first
    let mut vm = [0u64; 48]; // task_vm_info: resident_size at word 2, phys_footprint at word 19
    let mut count = (vm.len() * 2) as u32; // in natural_t (4 bytes)
    // SAFETY: getrusage(RUSAGE_SELF) into a buffer as large as struct rusage; task_info with
    // TASK_VM_INFO (22) into a buffer of `count` natural_t; mach_task_self_ is the task's port.
    let (r, k) = unsafe { (getrusage(0, ru.as_mut_ptr()), task_info(mach_task_self_, 22, vm.as_mut_ptr(), &mut count)) };
    let tv = |i: usize| ru[i * 2] * 1_000_000 + (ru[i * 2 + 1] & 0xffff_ffff);
    let cpu = if r == 0 { tv(0) + tv(1) } else { 0 };
    if k != 0 {
        return (cpu, 0, 0);
    }
    (cpu, vm[2], vm[19])
}

/// monotonic time in microseconds (CLOCK_MONOTONIC)
pub fn time_us() -> u64 {
    let mut ts = ffi::Timespec::default();
    // SAFETY: ts is a struct timespec.
    unsafe { ffi::clock_gettime(ffi::CLOCK_MONOTONIC, &mut ts) };
    ts.tv_sec as u64 * 1_000_000 + ts.tv_nsec as u64 / 1000
}

/// wall-clock time: nanoseconds since 1970-01-01 00:00 UTC (CLOCK_REALTIME)
pub fn unix_time_ns() -> i64 {
    let mut ts = ffi::Timespec::default();
    // SAFETY: ts is a struct timespec.
    unsafe { ffi::clock_gettime(ffi::CLOCK_REALTIME, &mut ts) };
    ts.tv_sec.wrapping_mul(1_000_000_000).wrapping_add(ts.tv_nsec)
}

// C string at p -> its bytes
unsafe fn cbytes<'a>(p: *const u8) -> &'a [u8] {
    let mut n = 0;
    // SAFETY: the caller hands a NUL-terminated string that outlives the slice.
    unsafe {
        while *p.add(n) != 0 {
            n += 1;
        }
        core::slice::from_raw_parts(p, n)
    }
}

/// the command line, argument 0 (the program) first
pub fn args() -> Vec<String> {
    let mut v = Vec::new();
    // SAFETY: libSystem's argc and argv of the process, valid for its whole life.
    unsafe {
        let argc = *ffi::_NSGetArgc();
        let argv = *ffi::_NSGetArgv();
        if argv.is_null() {
            return v;
        }
        for i in 0..argc.max(0) as usize {
            let a = *argv.add(i);
            if !a.is_null() {
                v.push(String::from_utf8_lossy(cbytes(a as *const u8)).into_owned());
            }
        }
    }
    v
}

// ---- environment: the process's block, copied on the first access so that setenv can replace
// entries (as linux.rs)
static ENV: rt::sync::SpinLock<Option<Vec<Vec<u8>>>> = rt::sync::SpinLock::new(None);

fn env_list(g: &mut Option<Vec<Vec<u8>>>) -> &mut Vec<Vec<u8>> {
    g.get_or_insert_with(|| {
        let mut v = Vec::new();
        // SAFETY: libSystem's environ: a NULL-terminated array of NUL-terminated strings.
        unsafe {
            let mut e = *ffi::_NSGetEnviron();
            if !e.is_null() {
                while !(*e).is_null() {
                    v.push(cbytes(*e as *const u8).to_vec());
                    e = e.add(1);
                }
            }
        }
        v
    })
}

/// the environment as NAME=VALUE entries (child processes)
#[allow(dead_code)]
pub(crate) fn process_environment() -> Vec<Vec<u8>> {
    env_list(&mut ENV.lock()).clone()
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

/// sleep for ms milliseconds (EINTR: the rest is slept)
pub fn sleep_ms(ms: u32) {
    let mut req = ffi::Timespec { tv_sec: (ms / 1000) as i64, tv_nsec: (ms % 1000) as i64 * 1_000_000 };
    let mut rem = ffi::Timespec::default();
    // SAFETY: two struct timespec; nanosleep writes the remaining time into rem on EINTR.
    while unsafe { ffi::nanosleep(&req, &mut rem) } != 0 && ffi::errno() == ffi::EINTR {
        req = rem;
    }
}

/// the path of this program's file (_NSGetExecutablePath; modhost.rs has it on Windows and Linux),
/// None when it cannot be read
pub fn current_exe() -> Option<String> {
    let mut buf = alloc::vec![0u8; 4096];
    let mut size = buf.len() as u32;
    // SAFETY: buf holds size bytes; the call writes a NUL-terminated path or fails.
    if unsafe { ffi::_NSGetExecutablePath(buf.as_mut_ptr().cast(), &mut size) } != 0 {
        return None;
    }
    let n = buf.iter().position(|&c| c == 0)?;
    Some(String::from_utf8_lossy(&buf[..n]).into_owned())
}
