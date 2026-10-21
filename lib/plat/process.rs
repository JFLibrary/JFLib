// Native child processes for tools and application helpers; no assembly counterpart.
// run_elevated / elevated: a program with administrator or root rights (a service install).
use alloc::string::String;
#[cfg(windows)]
#[path = "process_win.rs"]
mod native;
#[cfg(target_os = "linux")]
#[path = "process_linux.rs"]
mod native;
pub use native::Child;

/// Shell-free child invocation. Environment entries override inherited values.
pub struct Command<'a> {
    /// Executable path, or a program found in PATH.
    pub program: &'a str,
    /// Arguments excluding argv[0]; empty arguments are preserved.
    pub args: &'a [&'a str],
    /// Child working directory, or None to inherit.
    pub cwd: Option<&'a str>,
    /// Environment overrides; None removes a variable.
    pub env: &'a [(&'a str, Option<&'a str>)],
}
/// Run a program with administrator / root rights and wait for it -> its exit code.  Windows:
/// the UAC prompt (ShellExecuteEx "runas", hidden window, no pipes); Linux: directly when this
/// process is root, otherwise through pkexec (the desktop's password prompt).  Err when the
/// user refused or no way exists (the text says what to do).
pub fn run_elevated(program: &str, args: &[&str]) -> Result<i32, String> {
    if program.is_empty() || program.contains('\0') || args.iter().any(|a| a.contains('\0')) {
        return Err(String::from("ungültiger Aufruf"));
    }
    if !crate::policy::process_allowed(program) {
        return Err(String::from("vom Berechtigungs-Manifest (PERMISSIONS) nicht erlaubt"));
    }
    native::run_elevated(program, args)
}

/// Whether this process runs with administrator (Windows) / root (Linux) rights.
pub fn elevated() -> bool {
    native::elevated()
}

/// Replace this process by a program, keeping its PID, its terminal and its standard handles
/// (Linux: execve; argv[0] = program, a name without '/' is looked up in PATH, the environment
/// of this process with the overrides of env as in Command, SIGPIPE back to its default).
/// Returns only on failure: the error number (refused by the manifest: EACCES).  Windows has
/// no such call: 50 (ERROR_NOT_SUPPORTED).  The AppImage runtime (apps/appimagert) starts the
/// AppRun of the image with it.
pub fn exec(program: &str, args: &[&str], env: &[(&str, Option<&str>)]) -> u32 {
    if program.is_empty() || program.contains('\0') || args.iter().any(|a| a.contains('\0'))
        || env.iter().any(|(k, v)| k.is_empty() || k.contains(['=', '\0']) || v.is_some_and(|s| s.contains('\0')))
    { return 22; }
    if !crate::policy::process_allowed(program) {
        return if cfg!(windows) { 5 } else { 13 };
    }
    native::exec(program, args, env)
}

impl Command<'_> {
    /// Start a hidden process with pipes. Dropping its handle terminates its tree.
    pub fn spawn(&self) -> Result<Child, u32> {
        if self.program.is_empty() || self.program.contains('\0')
            || self.args.iter().any(|a| a.contains('\0'))
            || self.cwd.is_some_and(|p| p.contains('\0'))
            || self.env.iter().any(|(k, v)| k.is_empty() || k.contains(['=', '\0']) || v.is_some_and(|s| s.contains('\0')))
        { return Err(22); }
        if !crate::policy::process_allowed(self.program) {
            return Err(if cfg!(windows) { 5 } else { 13 }); // access denied / EACCES
        }
        native::spawn(self)
    }

    /// spawn without the manifest's check: the library's own helpers (assoc.rs: the
    /// desktop's cache tools); the caller passes clean arguments
    #[allow(dead_code)]
    pub(crate) fn spawn_own(&self) -> Result<Child, u32> {
        native::spawn(self)
    }
}
