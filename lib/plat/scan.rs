// scan -- pictures from a scanner through the system's own dialog
//
//   scan_available() -> bool      the system has a scan dialog here (macOS; elsewhere not yet)
//   scan_start(title) -> Result<(), ScanError>   the dialog in a window titled `title`: the user
//                                  chooses the device, previews, sets the area and the resolution
//                                  and scans; Busy while one is open, Unavailable where there is
//                                  none.  Call it from the UI thread.
//   scan_poll() -> Option<Result<Scanned, ScanError>>   None while the dialog is open (or none
//                                  was), then once: the scanned file, Cancelled (the window closed
//                                  without a scan) or Failed (the device or the system)
//   scan_open() -> bool            a dialog is open (its answer not taken yet)
//
// Scanned: the file as the scanner wrote it -- its name and bytes (macOS: PNG unless the user
// chose another format in the dialog); the app decodes it like a file it opened.  The answer
// wakes the UI (plat::wake).  Scanning needs no right of the app's manifest: the user starts it
// and chooses in the system's dialog, as in the file dialog.
//
// macOS (apple_mac_scan.rs): ImageKit's device browser (local and network scanners: USB,
// AirScan / eSCL, Bonjour) beside its scanner view (IKScannerDeviceView over ImageCaptureCore,
// the dialog Preview shows), the scan written to a folder of its own in the temp folder, read
// and removed.  An app under the App Sandbox needs the USB and network entitlements for it.
// Windows (WIA) and Linux (SANE) follow; until then Unavailable there, in the browser and on
// Android and iOS.

use alloc::string::String;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, Ordering};

use rt::sync::SpinLock;

/// why no picture came
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ScanError {
    /// no scan dialog on this system
    Unavailable,
    /// the user closed the dialog without a scan
    Cancelled,
    /// a dialog is open already
    Busy,
    /// the device or the system failed: what it said
    Failed(String),
}

impl core::fmt::Display for ScanError {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        match self {
            ScanError::Unavailable => f.write_str("not available here"),
            ScanError::Cancelled => f.write_str("cancelled"),
            ScanError::Busy => f.write_str("busy"),
            ScanError::Failed(s) => write!(f, "failed: {s}"),
        }
    }
}

/// a scanned picture: the file as the scanner wrote it
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Scanned {
    /// its name ("Scan.png")
    pub name: String,
    /// its bytes
    pub data: Vec<u8>,
}

static ANSWER: SpinLock<Option<Result<Scanned, ScanError>>> = SpinLock::new(None);
static OPEN: AtomicBool = AtomicBool::new(false);

/// (backends) the dialog's answer: kept for scan_poll, the UI woken
#[allow(dead_code)]
pub(crate) fn answer(r: Result<Scanned, ScanError>) {
    *ANSWER.lock() = Some(r);
    crate::wake();
}

/// whether the system has a scan dialog here
pub fn scan_available() -> bool {
    native::available()
}

/// open the system's scan dialog in a window titled `title`; the answer comes through scan_poll
pub fn scan_start(title: &str) -> Result<(), ScanError> {
    if !native::available() {
        return Err(ScanError::Unavailable);
    }
    if OPEN.swap(true, Ordering::AcqRel) {
        return Err(ScanError::Busy);
    }
    *ANSWER.lock() = None;
    let title: String = title.chars().filter(|&c| c != '\0').take(200).collect();
    let r = native::start(&title);
    if r.is_err() {
        *ANSWER.lock() = None;
        OPEN.store(false, Ordering::Release);
    }
    r
}

/// the open dialog's answer: None while it is open (or none was), then once
pub fn scan_poll() -> Option<Result<Scanned, ScanError>> {
    let r = ANSWER.lock().take()?;
    OPEN.store(false, Ordering::Release);
    Some(r)
}

/// a dialog is open (its answer not taken yet)
pub fn scan_open() -> bool {
    OPEN.load(Ordering::Acquire)
}

#[cfg(target_os = "macos")]
mod native {
    pub use crate::apple::mac::scan_dialog::{available, start};
}

#[cfg(not(target_os = "macos"))]
mod native {
    use super::ScanError;
    pub fn available() -> bool {
        false
    }
    pub fn start(_title: &str) -> Result<(), ScanError> {
        Err(ScanError::Unavailable)
    }
}
