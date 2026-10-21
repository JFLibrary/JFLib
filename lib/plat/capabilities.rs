// Platform service availability, shared by app commands, agent schemas and browser hosts.

/// Read files through the platform's file namespace.
pub const CAP_FILE_READ: u64 = 1 << 0;
/// Write files through the platform's file namespace.
pub const CAP_FILE_WRITE: u64 = 1 << 1;
/// Confirm durable storage, rather than a temporary in-memory write.
pub const CAP_PERSISTENT_STORAGE: u64 = 1 << 2;
/// Exchange plain UTF-8 clipboard text.
pub const CAP_CLIPBOARD_TEXT: u64 = 1 << 3;
/// Exchange native/RTF/HTML rich clipboard representations.
/// Browser hosts provide an instance-local clipboard, without OS rich-format access.
pub const CAP_CLIPBOARD_RICH: u64 = 1 << 4;
/// Exchange clipboard images.
pub const CAP_CLIPBOARD_IMAGE: u64 = 1 << 5;
/// Submit a print job through the host.
pub const CAP_PRINT: u64 = 1 << 6;
/// Discover fonts installed in the operating system.
pub const CAP_SYSTEM_FONTS: u64 = 1 << 7;
/// Load native shared libraries with the platform plugin ABI.
pub const CAP_NATIVE_PLUGINS: u64 = 1 << 8;
/// Connect through a user-local IPC transport after explicit activation.
pub const CAP_LOCAL_IPC: u64 = 1 << 9;
/// Display windows, including library windows inside a browser canvas.
pub const CAP_WINDOWING: u64 = 1 << 10;
/// Run parallel operating-system threads.
pub const CAP_THREADS: u64 = 1 << 11;
/// Install operating-system shortcuts or integrations.
pub const CAP_SHELL_INTEGRATION: u64 = 1 << 12;

/// Snapshot of services actually provided by the current host.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Capabilities {
    /// Bit set of the `CAP_*` services.
    pub available: u64,
}
impl Capabilities {
    /// Whether every requested service is available.
    pub const fn supports(self, required: u64) -> bool {
        self.available & required == required
    }
    /// A concrete explanation for the first missing service, or no limitation.
    pub fn unavailable_reason(self, required: u64) -> Option<&'static str> {
        let missing = required & !self.available;
        let reasons = [
            (CAP_FILE_READ, "This host has not granted file access."),
            (CAP_FILE_WRITE, "This host has not granted file write access."),
            (CAP_PERSISTENT_STORAGE, "Persistent storage has not been granted or confirmed."),
            (CAP_CLIPBOARD_TEXT, "Clipboard text access is not available or has not been granted."),
            (CAP_CLIPBOARD_RICH, "This host cannot exchange rich clipboard formats."),
            (CAP_CLIPBOARD_IMAGE, "This host cannot exchange clipboard images."),
            (CAP_PRINT, "Printing is not available on this host."),
            (CAP_SYSTEM_FONTS, "Installed system fonts are not accessible on this host."),
            (CAP_NATIVE_PLUGINS, "Native DLL/SO plugins cannot run on this platform."),
            (CAP_LOCAL_IPC, "Local IPC requires a native host or an explicitly connected browser bridge."),
            (CAP_WINDOWING, "No window surface is available."),
            (CAP_THREADS, "This host uses sequential execution instead of operating-system threads."),
            (CAP_SHELL_INTEGRATION, "Operating-system installation is unavailable on this host."),
        ];
        reasons.iter().find(|(flag, _)| missing & flag != 0).map(|(_, reason)| *reason)
            .or(if missing != 0 { Some("An unknown platform capability was requested.") } else { None })
    }
}

/// Current native service set, less what the app's permissions leave out (policy.rs).
/// Browser permissions are supplied by its host adapter.
#[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
pub fn capabilities() -> Capabilities {
    Capabilities { available: ((1 << 13) - 1) & crate::policy::cap_mask() }
}
