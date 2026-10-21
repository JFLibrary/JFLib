// service -- programs that run as a system service
//
//   run_service(name, run) -> exit code
//       Windows: started by the service control manager, run() runs while the service reports
//       RUNNING; a stop or shutdown request sets stop_requested(), reports STOP_PENDING and gives
//       run() 20 s before the process ends.  Started any other way (a console): run() directly.
//       Linux: run() directly (systemd starts the program like any other; SIGTERM ends it, the
//       programs keep their data crash-safe).
//   stop_requested() -> bool
//   install(&ServiceSpec) / uninstall(name) / start(name) / stop(name) / state(name)
//       Windows: the service control manager (automatic start as LocalSystem, restart after a
//       failure); Linux: a systemd unit in /etc/systemd/system and systemctl.  Both need
//       administrator / root rights; the errors say so.
//   unit_text(&ServiceSpec) -> the systemd unit install writes (Linux)

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, Ordering};

static STOP: AtomicBool = AtomicBool::new(false);

/// whether the service manager asked the program to stop
pub fn stop_requested() -> bool {
    STOP.load(Ordering::Acquire)
}

/// what a service is
pub struct ServiceSpec<'a> {
    /// Windows: the service name ("BackupCraftServer"); Linux: the unit name ("backupcraft-server")
    pub name: &'a str,
    pub display: &'a str,
    pub description: &'a str,
    /// the program (absolute) and its arguments
    pub exe: &'a str,
    pub args: &'a [String],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServiceState {
    NotInstalled,
    Stopped,
    Starting,
    Running,
    Stopping,
    Other,
}

impl ServiceState {
    pub fn text(self) -> &'static str {
        match self {
            ServiceState::NotInstalled => "nicht eingerichtet",
            ServiceState::Stopped => "angehalten",
            ServiceState::Starting => "startet",
            ServiceState::Running => "läuft",
            ServiceState::Stopping => "hält an",
            ServiceState::Other => "unbekannt",
        }
    }
}

/// one argument of an ExecStart line (systemd quoting)
fn unit_arg(a: &str) -> String {
    let e = a.replace('%', "%%").replace('$', "$$");
    if !e.is_empty() && !e.contains([' ', '\t', '"', '\'', '\\', ';']) {
        return e;
    }
    format!("\"{}\"", e.replace('\\', "\\\\").replace('"', "\\\""))
}

/// the systemd unit of a service
pub fn unit_text(s: &ServiceSpec) -> String {
    let mut exec = unit_arg(s.exe);
    for a in s.args {
        exec.push(' ');
        exec.push_str(&unit_arg(a));
    }
    format!(
        "[Unit]\nDescription={}\nAfter=network-online.target\nWants=network-online.target\n\n[Service]\nType=simple\nExecStart={}\nRestart=on-failure\nRestartSec=5\nUMask=0077\n\n[Install]\nWantedBy=multi-user.target\n",
        s.description.replace('\n', " "),
        exec
    )
}

#[cfg(windows)]
pub use win::*;
#[cfg(target_os = "linux")]
pub use linux::*;

#[cfg(windows)]
mod win {
    use super::*;
    use alloc::boxed::Box;
    use core::ffi::c_void;
    use core::sync::atomic::{AtomicI32, AtomicIsize, AtomicPtr, AtomicUsize};

    #[repr(C)]
    #[derive(Default, Clone, Copy)]
    struct Status {
        kind: u32,
        state: u32,
        accepted: u32,
        exit: u32,
        specific: u32,
        checkpoint: u32,
        wait_hint: u32,
    }

    #[repr(C)]
    struct TableEntry {
        name: *const u16,
        proc_: Option<unsafe extern "system" fn(u32, *mut *mut u16)>,
    }

    #[repr(C)]
    struct Description {
        text: *const u16,
    }

    #[repr(C)]
    struct Action {
        kind: u32,
        delay: u32,
    }

    #[repr(C)]
    struct FailureActions {
        reset_period: u32,
        reboot_msg: *const u16,
        command: *const u16,
        count: u32,
        actions: *const Action,
    }

    #[link(name = "advapi32", kind = "raw-dylib")]
    unsafe extern "system" {
        fn StartServiceCtrlDispatcherW(table: *const TableEntry) -> i32;
        fn RegisterServiceCtrlHandlerExW(name: *const u16, handler: unsafe extern "system" fn(u32, u32, *mut c_void, *mut c_void) -> u32, ctx: *mut c_void) -> isize;
        fn SetServiceStatus(h: isize, status: *const Status) -> i32;
        fn OpenSCManagerW(machine: *const u16, db: *const u16, access: u32) -> isize;
        fn CreateServiceW(scm: isize, name: *const u16, display: *const u16, access: u32, kind: u32, start: u32, error: u32, path: *const u16, group: *const u16, tag: *mut u32, deps: *const u16, user: *const u16, password: *const u16) -> isize;
        fn OpenServiceW(scm: isize, name: *const u16, access: u32) -> isize;
        fn ChangeServiceConfig2W(h: isize, level: u32, info: *const c_void) -> i32;
        fn StartServiceW(h: isize, argc: u32, argv: *const *const u16) -> i32;
        fn ControlService(h: isize, control: u32, status: *mut Status) -> i32;
        fn QueryServiceStatus(h: isize, status: *mut Status) -> i32;
        fn DeleteService(h: isize) -> i32;
        fn CloseServiceHandle(h: isize) -> i32;
    }
    #[link(name = "kernel32", kind = "raw-dylib")]
    unsafe extern "system" {
        fn GetLastError() -> u32;
    }

    const OWN_PROCESS: u32 = 0x10;
    const STOPPED: u32 = 1;
    const START_PENDING: u32 = 2;
    const STOP_PENDING: u32 = 3;
    const RUNNING: u32 = 4;
    const ACCEPT_STOP_SHUTDOWN: u32 = 0x1 | 0x4;

    static NAME: AtomicPtr<u16> = AtomicPtr::new(core::ptr::null_mut());
    static RUN: AtomicUsize = AtomicUsize::new(0);
    static HANDLE: AtomicIsize = AtomicIsize::new(0);
    static EXIT: AtomicI32 = AtomicI32::new(0);

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(core::iter::once(0)).collect()
    }

    fn report(state: u32, exit: u32, wait_hint: u32) {
        let st = Status { kind: OWN_PROCESS, state, accepted: if state == RUNNING { ACCEPT_STOP_SHUTDOWN } else { 0 }, exit, specific: 0, checkpoint: 0, wait_hint };
        let h = HANDLE.load(Ordering::Acquire);
        if h != 0 {
            // SAFETY: a status handle from RegisterServiceCtrlHandlerExW and a valid record.
            unsafe { SetServiceStatus(h, &st) };
        }
    }

    unsafe extern "system" fn handler(control: u32, _event: u32, _data: *mut c_void, _ctx: *mut c_void) -> u32 {
        match control {
            // stop, shutdown
            1 | 5 => {
                if !STOP.swap(true, Ordering::AcqRel) {
                    report(STOP_PENDING, 0, 20_000);
                    // run() gets 20 s, then the process ends anyway
                    let _ = crate::thread_spawn(64 * 1024, Box::new(|| {
                        crate::sleep_ms(20_000);
                        report(STOPPED, 0, 0);
                        crate::exit(0);
                    }));
                }
                0
            }
            // interrogate
            4 => 0,
            _ => 120, // ERROR_CALL_NOT_IMPLEMENTED
        }
    }

    unsafe extern "system" fn service_main(_argc: u32, _argv: *mut *mut u16) {
        // SAFETY: NAME points at a leaked, NUL-terminated wide string; the handler lives forever.
        let h = unsafe { RegisterServiceCtrlHandlerExW(NAME.load(Ordering::Acquire), handler, core::ptr::null_mut()) };
        HANDLE.store(h, Ordering::Release);
        report(START_PENDING, 0, 5000);
        report(RUNNING, 0, 0);
        let f = RUN.load(Ordering::Acquire);
        // run() on a thread of our own (the runtime's thread setup), joined here
        let t = crate::thread_spawn(1 << 20, Box::new(move || {
            // SAFETY: RUN holds the fn() -> i32 run_service stored.
            let run: fn() -> i32 = unsafe { core::mem::transmute::<usize, fn() -> i32>(f) };
            EXIT.store(run(), Ordering::Release);
        }));
        if let Some(t) = t {
            t.join();
        }
        let code = EXIT.load(Ordering::Acquire);
        report(STOPPED, if code == 0 { 0 } else { 1066 }, 0);
    }

    pub fn run_service(name: &str, run: fn() -> i32) -> i32 {
        let w = Box::leak(wide(name).into_boxed_slice());
        NAME.store(w.as_mut_ptr(), Ordering::Release);
        RUN.store(run as usize, Ordering::Release);
        let table = [TableEntry { name: w.as_ptr(), proc_: Some(service_main) }, TableEntry { name: core::ptr::null(), proc_: None }];
        // SAFETY: a NUL-terminated table that outlives the dispatcher (which returns when the
        // service stopped).
        if unsafe { StartServiceCtrlDispatcherW(table.as_ptr()) } == 0 {
            // SAFETY: no arguments.
            let e = unsafe { GetLastError() };
            if e == 1063 {
                return run(); // not started by the service control manager
            }
            return 1;
        }
        EXIT.load(Ordering::Acquire)
    }

    fn error(what: &str) -> String {
        // SAFETY: no arguments.
        let e = unsafe { GetLastError() };
        let why = match e {
            5 => String::from("Zugriff verweigert – als Administrator ausführen"),
            1056 => String::from("der Dienst läuft bereits"),
            1060 => String::from("der Dienst ist nicht eingerichtet"),
            1062 => String::from("der Dienst läuft nicht"),
            1072 => String::from("der Dienst ist zum Löschen vorgemerkt – bitte kurz warten"),
            1073 => String::from("der Dienst ist bereits eingerichtet"),
            e => format!("Fehler {}", e),
        };
        format!("{}: {}", what, why)
    }

    struct Handle(isize);
    impl Drop for Handle {
        fn drop(&mut self) {
            if self.0 != 0 {
                // SAFETY: a handle of the service control manager.
                unsafe { CloseServiceHandle(self.0) };
            }
        }
    }

    fn scm(access: u32) -> Result<Handle, String> {
        // SAFETY: the local machine's active database.
        let h = unsafe { OpenSCManagerW(core::ptr::null(), core::ptr::null(), access) };
        if h == 0 { Err(error("Dienstverwaltung")) } else { Ok(Handle(h)) }
    }

    fn open(name: &str, access: u32) -> Result<Handle, String> {
        let m = scm(0x0001)?; // connect
        let w = wide(name);
        // SAFETY: a valid manager handle and wide name.
        let h = unsafe { OpenServiceW(m.0, w.as_ptr(), access) };
        if h == 0 { Err(error(name)) } else { Ok(Handle(h)) }
    }

    /// "exe" args, quoted for CommandLineToArgvW
    fn command_line(exe: &str, args: &[String]) -> String {
        let q = |a: &str| -> String {
            if !a.is_empty() && !a.contains([' ', '\t', '"']) {
                return String::from(a);
            }
            let mut s = String::from("\"");
            let mut slashes = 0;
            for c in a.chars() {
                match c {
                    '\\' => slashes += 1,
                    '"' => {
                        s.extend(core::iter::repeat_n('\\', slashes * 2 + 1));
                        s.push('"');
                        slashes = 0;
                        continue;
                    }
                    _ => {}
                }
                if c != '\\' {
                    s.extend(core::iter::repeat_n('\\', slashes));
                    slashes = 0;
                    s.push(c);
                }
            }
            s.extend(core::iter::repeat_n('\\', slashes * 2));
            s.push('"');
            s
        };
        let mut s = format!("\"{}\"", exe);
        for a in args {
            s.push(' ');
            s.push_str(&q(a));
        }
        s
    }

    pub fn install(s: &ServiceSpec) -> Result<(), String> {
        let m = scm(0x0002 | 0x0001)?; // create service, connect
        let (name, display, path) = (wide(s.name), wide(s.display), wide(&command_line(s.exe, s.args)));
        // SAFETY: wide strings that live through the call; LocalSystem (no account).
        let h = unsafe {
            CreateServiceW(m.0, name.as_ptr(), display.as_ptr(), 0xF01FF, OWN_PROCESS, 2, 1, path.as_ptr(), core::ptr::null(), core::ptr::null_mut(), core::ptr::null(), core::ptr::null(), core::ptr::null())
        };
        if h == 0 {
            return Err(error(s.name));
        }
        let h = Handle(h);
        let text = wide(s.description);
        let d = Description { text: text.as_ptr() };
        let actions = [Action { kind: 1, delay: 5000 }, Action { kind: 1, delay: 15_000 }, Action { kind: 1, delay: 60_000 }];
        let fa = FailureActions { reset_period: 86_400, reboot_msg: core::ptr::null(), command: core::ptr::null(), count: 3, actions: actions.as_ptr() };
        // SAFETY: records that live through the calls.
        unsafe {
            ChangeServiceConfig2W(h.0, 1, &d as *const Description as *const c_void);
            ChangeServiceConfig2W(h.0, 2, &fa as *const FailureActions as *const c_void);
        }
        Ok(())
    }

    pub fn start(name: &str) -> Result<(), String> {
        let h = open(name, 0x0010)?;
        // SAFETY: a service handle, no arguments.
        if unsafe { StartServiceW(h.0, 0, core::ptr::null()) } == 0 { Err(error(name)) } else { Ok(()) }
    }

    pub fn stop(name: &str) -> Result<(), String> {
        let h = open(name, 0x0020 | 0x0004)?;
        let mut st = Status::default();
        // SAFETY: a service handle and a status record.
        if unsafe { ControlService(h.0, 1, &mut st) } == 0 {
            return Err(error(name));
        }
        for _ in 0..300 {
            // SAFETY: as above.
            if unsafe { QueryServiceStatus(h.0, &mut st) } == 0 || st.state == STOPPED {
                break;
            }
            crate::sleep_ms(100);
        }
        Ok(())
    }

    pub fn uninstall(name: &str) -> Result<(), String> {
        let _ = stop(name);
        let h = open(name, 0x10000)?; // delete
        // SAFETY: a service handle.
        if unsafe { DeleteService(h.0) } == 0 { Err(error(name)) } else { Ok(()) }
    }

    pub fn state(name: &str) -> ServiceState {
        let Ok(h) = open(name, 0x0004) else { return ServiceState::NotInstalled };
        let mut st = Status::default();
        // SAFETY: a service handle and a status record.
        if unsafe { QueryServiceStatus(h.0, &mut st) } == 0 {
            return ServiceState::Other;
        }
        match st.state {
            STOPPED => ServiceState::Stopped,
            START_PENDING => ServiceState::Starting,
            RUNNING => ServiceState::Running,
            STOP_PENDING => ServiceState::Stopping,
            _ => ServiceState::Other,
        }
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use super::*;

    pub fn run_service(_name: &str, run: fn() -> i32) -> i32 {
        run()
    }

    fn is_root() -> bool {
        // SAFETY: getuid has no arguments and cannot fail.
        unsafe { rt::os::syscall3(102, 0, 0, 0) == 0 }
    }

    /// systemctl ARGS -> its output, or why it failed
    fn systemctl(args: &[&str]) -> Result<String, String> {
        match systemctl_run(args)? {
            (0, out, _) => Ok(out),
            (_, _, err) => Err(format!("systemctl {}: {}", args.join(" "), err)),
        }
    }

    /// systemctl ARGS -> exit code, stdout, stderr
    fn systemctl_run(args: &[&str]) -> Result<(i32, String, String), String> {
        let c = crate::process::Command { program: "systemctl", args, cwd: None, env: &[] };
        let mut ch = c.spawn().map_err(|e| format!("systemctl kann nicht gestartet werden (Fehler {})", e))?;
        ch.close_stdin();
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let mut buf = [0u8; 4096];
        for _ in 0..6000 {
            let mut busy = false;
            while let Ok(Some(n)) = ch.read_stdout(&mut buf) {
                if n == 0 {
                    break;
                }
                out.extend_from_slice(&buf[..n]);
                busy = true;
            }
            while let Ok(Some(n)) = ch.read_stderr(&mut buf) {
                if n == 0 {
                    break;
                }
                err.extend_from_slice(&buf[..n]);
                busy = true;
            }
            if let Ok(Some(code)) = ch.try_wait() {
                let o = String::from(String::from_utf8_lossy(&out).trim());
                let e = String::from(String::from_utf8_lossy(&err).trim());
                return Ok((code, o, e));
            }
            if !busy {
                crate::sleep_ms(10);
            }
        }
        Err(format!("systemctl {}: keine Antwort", args.join(" ")))
    }

    fn unit_path(name: &str) -> String {
        format!("/etc/systemd/system/{}.service", name)
    }

    fn need_root() -> Result<(), String> {
        if is_root() { Ok(()) } else { Err(String::from("nur als root (sudo) möglich")) }
    }

    pub fn install(s: &ServiceSpec) -> Result<(), String> {
        need_root()?;
        if crate::fs::stat("/run/systemd/system").is_none() {
            return Err(String::from("systemd läuft auf diesem Rechner nicht"));
        }
        let p = unit_path(s.name);
        if crate::fs::stat(&p).is_some() {
            return Err(format!("{}: der Dienst ist bereits eingerichtet", s.name));
        }
        if !crate::write_file(&p, unit_text(s).as_bytes()) {
            return Err(format!("{} kann nicht geschrieben werden", p));
        }
        systemctl(&["daemon-reload"])?;
        systemctl(&["enable", s.name]).map(|_| ())
    }

    pub fn start(name: &str) -> Result<(), String> {
        need_root()?;
        systemctl(&["start", name]).map(|_| ())
    }

    pub fn stop(name: &str) -> Result<(), String> {
        need_root()?;
        systemctl(&["stop", name]).map(|_| ())
    }

    pub fn uninstall(name: &str) -> Result<(), String> {
        need_root()?;
        let p = unit_path(name);
        if crate::fs::stat(&p).is_none() {
            return Err(format!("{}: der Dienst ist nicht eingerichtet", name));
        }
        let _ = systemctl(&["disable", "--now", name]);
        crate::delete(&p);
        systemctl(&["daemon-reload"]).map(|_| ())
    }

    pub fn state(name: &str) -> ServiceState {
        if crate::fs::stat(&unit_path(name)).is_none() && crate::fs::stat(&format!("/lib/systemd/system/{}.service", name)).is_none() {
            return ServiceState::NotInstalled;
        }
        let s = systemctl_run(&["is-active", name]).map(|(_, out, _)| out).unwrap_or_default();
        match s.trim() {
            "active" => ServiceState::Running,
            "inactive" | "failed" => ServiceState::Stopped,
            "activating" => ServiceState::Starting,
            "deactivating" => ServiceState::Stopping,
            _ => ServiceState::Other,
        }
    }
}
