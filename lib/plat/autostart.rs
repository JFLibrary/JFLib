// autostart -- start the app when the user logs in
//
//   autostart_set(on, args) -> Result<(), AutostartError>   on: start this program with args
//                                     at login (an existing entry is replaced); off: remove it
//   autostart_get() -> bool           an entry of this app is there and switched on
//   autostart_entry() -> Option<String>   the command the entry starts (Windows: the Run
//                                     value, Linux: the Exec value, Flatpak: what was asked
//                                     last), None without one
//
// Changing it needs `autostart` in the app's manifest (policy.rs; else Denied and "PERMDENY
// autostart"); reading needs nothing.  The entry is named after the app (vfs::app_id; Linux:
// the .desktop id, linux_desk.rs) and starts the running executable -- call autostart_set
// again after the program moved (autostart_entry tells where the entry points).
//
// Windows (win_autostart.rs): the value <app id> = "exe" args of
// HKCU\Software\Microsoft\Windows\CurrentVersion\Run (arguments quoted with the Microsoft
// rules).  The Task Manager's switch (Explorer\StartupApproved\Run, an odd first byte = off)
// counts for autostart_get; autostart_set(true) clears it (the user asked in the app), false
// removes both; in an MSIX package (HKCU virtualized) Unavailable: the package's StartupTask
// is switched in Settings.  Linux (linux_autostart.rs): $XDG_CONFIG_HOME/autostart/<desktop id>.desktop
// (XDG autostart; Hidden=true or X-GNOME-Autostart-enabled=false count as off), Exec quoted
// as the Desktop Entry spec wants, $APPIMAGE instead of the executable when set.  In Flatpak
// the Background portal (RequestBackground with autostart and the command line): the desktop
// may ask the user, and the call waits for the answer (at most AUTOSTART_WAIT_MS); the sandbox
// cannot see the host's autostart folder, so the answer is remembered in
// config://autostart.  jfgui_AUTOSTART=portal / file picks the way (tests).  Web and
// Android: Unavailable.

use alloc::string::String;

/// how long autostart_set waits for the Background portal's answer (the user may be asked)
pub const AUTOSTART_WAIT_MS: i32 = 120_000;

/// why autostart_set failed
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AutostartError {
    /// not possible here (web, Android, no session bus for the portal, an MSIX package: its
    /// StartupTask instead)
    Unavailable,
    /// the app's permissions lack `autostart`
    Denied,
    /// the user (or the desktop) refused it, or did not answer in time (the portal)
    Dismissed,
    /// an argument with NUL, or a command line beyond the system's limit
    Invalid,
    /// the system refused or failed: what it said
    Failed(String),
}

impl core::fmt::Display for AutostartError {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        match self {
            AutostartError::Unavailable => f.write_str("autostart is not available here"),
            AutostartError::Denied => f.write_str("the app's permissions do not allow autostart"),
            AutostartError::Dismissed => f.write_str("autostart was refused"),
            AutostartError::Invalid => f.write_str("invalid autostart arguments"),
            AutostartError::Failed(s) => write!(f, "autostart: {s}"),
        }
    }
}

/// start this program with args when the user logs in (on), or not any more (off)
pub fn autostart_set(on: bool, args: &[&str]) -> Result<(), AutostartError> {
    if args.iter().any(|a| a.contains('\0')) {
        return Err(AutostartError::Invalid);
    }
    if !crate::policy::autostart_allowed() {
        return Err(AutostartError::Denied);
    }
    native::set(on, args)
}

/// whether the app starts at login (an entry of it is there and switched on)
pub fn autostart_get() -> bool {
    native::get()
}

/// the command the app's entry starts, None without one
pub fn autostart_entry() -> Option<String> {
    native::entry()
}

#[cfg(windows)]
#[path = "win_autostart.rs"]
pub(crate) mod native;

#[cfg(target_os = "linux")]
#[path = "linux_autostart.rs"]
pub(crate) mod native;

#[cfg(not(any(windows, target_os = "linux")))]
mod native {
    use super::AutostartError;
    use alloc::string::String;

    pub fn set(_on: bool, _args: &[&str]) -> Result<(), AutostartError> {
        Err(AutostartError::Unavailable)
    }
    pub fn get() -> bool {
        false
    }
    pub fn entry() -> Option<String> {
        None
    }
}
