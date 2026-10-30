// pkg -- the package the running program comes from, and what its package system offers for
// updates (lib/update builds on it; docs/UPDATES.md "Packages")
//
//   packaged() -> Packaged          Msix(full name) (GetCurrentPackageFullName), Flatpak(app id)
//                                   (/.flatpak-info; $FLATPAK_ID), AppImage(file) ($APPIMAGE, an
//                                   absolute path), Snap ($SNAP), else Plain
//   msix_updates() -> Result<MsixUpdates, String>   (Windows, in an MSIX package) the App
//                                   Installer file the package came from (Package
//                                   .GetAppInstallerInfo: its address) and, with one, whether it
//                                   offers a newer version (Package.CheckUpdateAvailabilityAsync);
//                                   WinRT through raw COM (win_winrt.rs)
//   FlatpakMonitor                  (Linux, in a Flatpak) the portal's update monitor
//                                   (linux_flatpak.rs): create(), poll(ms) -> UpdateAvailable,
//                                   update(progress, cancel) -> the newest version installed
//   flatpak_spawn_latest(args)      (Linux) the app's newest installed version started with args
//                                   (the portal's Spawn, LATEST_VERSION) -> its pid
//   restart_appimage(args) -> i32   (Linux) the AppImage file started again with args (its
//                                   runtime unpacks the image that is now there): 0 / -1
//
// Elsewhere (web, Android) packaged() is Plain and the rest answers "not available".

use alloc::string::String;

/// where the running program comes from
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Packaged {
    /// a program file of its own: a setup, a .deb or .rpm, a portable copy
    Plain,
    /// an AppImage: the .AppImage file (the program runs from a copy its runtime unpacked)
    AppImage(String),
    /// a Flatpak: the app id (the program's folder is the read-only /app)
    Flatpak(String),
    /// an MSIX package: its full name (the install folder is read-only)
    Msix(String),
    /// a Snap (snapd updates it)
    Snap,
}

/// the package the running program comes from
pub fn packaged() -> Packaged {
    #[cfg(windows)]
    if let Some(name) = msix::full_name() {
        return Packaged::Msix(name);
    }
    #[cfg(target_os = "linux")]
    {
        let set = |v: &str| crate::env(v).filter(|x| !x.is_empty());
        if crate::linux_desk::in_flatpak() || set("FLATPAK_ID").is_some() || crate::env("container").as_deref() == Some("flatpak") {
            return Packaged::Flatpak(set("FLATPAK_ID").unwrap_or_default());
        }
        if let Some(p) = set("APPIMAGE").filter(|p| p.starts_with('/')) {
            return Packaged::AppImage(p);
        }
        if set("SNAP").is_some() {
            return Packaged::Snap;
        }
    }
    Packaged::Plain
}

/// what App Installer knows about the running MSIX package (msix_updates)
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MsixUpdates {
    /// the address of the .appinstaller file the package was installed from ("": none -- the
    /// Store, Add-AppxPackage, a developer registration)
    pub appinstaller: String,
    /// Package.CheckUpdateAvailabilityAsync with an .appinstaller: 0 unknown, 1 no updates,
    /// 2 available, 3 required, 4 error; without one 0
    pub availability: u32,
}

/// the App Installer file of the running MSIX package and whether it offers an update
/// (Windows 10 1809 and later) -> Err: not in a package, or WinRT failed (its HRESULT)
pub fn msix_updates() -> Result<MsixUpdates, String> {
    #[cfg(windows)]
    {
        msix::updates()
    }
    #[cfg(not(windows))]
    {
        Err(String::from("no MSIX package"))
    }
}

/// UpdateAvailable of the Flatpak portal: the commits of the running, the installed and the
/// remote version (an installed one newer than the running one: restart to use it)
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FlatpakUpdate {
    pub running: String,
    pub local: String,
    pub remote: String,
}

#[cfg(target_os = "linux")]
#[path = "linux_flatpak.rs"]
mod flatpak;
#[cfg(target_os = "linux")]
pub use flatpak::FlatpakMonitor;

/// (not Linux) there is no Flatpak portal: create() fails
#[cfg(not(target_os = "linux"))]
pub struct FlatpakMonitor;

#[cfg(not(target_os = "linux"))]
impl FlatpakMonitor {
    pub fn create() -> Result<FlatpakMonitor, String> {
        Err(String::from("no Flatpak"))
    }
    pub fn poll(&mut self, _ms: i32) -> Option<FlatpakUpdate> {
        None
    }
    pub fn update(&mut self, _progress: &mut dyn FnMut(u32, u32, u32), _cancel: &dyn Fn() -> bool) -> Result<(), String> {
        Err(String::from("no Flatpak"))
    }
}

/// start this app's newest installed Flatpak version with args (the portal's Spawn with
/// LATEST_VERSION; the same program inside it) -> its pid
pub fn flatpak_spawn_latest(args: &[&str]) -> Result<u32, String> {
    #[cfg(target_os = "linux")]
    {
        flatpak::spawn_latest(args)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = args;
        Err(String::from("no Flatpak"))
    }
}

/// start the running AppImage's file ($APPIMAGE) again with args: its runtime unpacks the
/// image the file holds now (after an update) and runs it -> 0, -1 when not an AppImage or no
/// process could be started
pub fn restart_appimage(args: &[&str]) -> i32 {
    #[cfg(target_os = "linux")]
    {
        match packaged() {
            Packaged::AppImage(p) => crate::spawn_file(&p, args),
            _ => -1,
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = args;
        -1
    }
}

#[cfg(windows)]
mod msix {
    use alloc::format;
    use alloc::string::String;
    use alloc::vec::Vec;

    use super::MsixUpdates;
    use crate::win_winrt::{self, Guid, Obj};

    #[link(name = "kernel32", kind = "raw-dylib")]
    unsafe extern "system" {
        fn GetCurrentPackageFullName(len: *mut u32, name: *mut u16) -> i32;
    }

    static IID_IPACKAGESTATICS: Guid = Guid(0x4E53_4BDF, 0x2960, 0x4878, [0x97, 0xA4, 0x96, 0x24, 0xDE, 0xB7, 0x2F, 0x2D]);
    static IID_IPACKAGE6: Guid = Guid(0x8B1A_D942, 0x12D7, 0x4754, [0xAE, 0x4E, 0x63, 0x8C, 0xBC, 0x0E, 0x3A, 0x2E]);
    // IPackageStatics.Current; IPackage6.GetAppInstallerInfo / CheckUpdateAvailabilityAsync;
    // IAppInstallerInfo.Uri; IUriRuntimeClass.AbsoluteUri; IAsyncOperation<T>.GetResults;
    // IPackageUpdateAvailabilityResult.Availability
    const PS_CURRENT: usize = 6;
    const P6_APPINSTALLERINFO: usize = 6;
    const P6_CHECKUPDATE: usize = 7;
    const AI_URI: usize = 6;
    const URI_ABSOLUTE: usize = 6;
    const OP_RESULTS: usize = 8;
    const UR_AVAILABILITY: usize = 6;
    const CHECK_MS: u32 = 60_000;
    const ERROR_INSUFFICIENT_BUFFER: i32 = 122;

    /// the package's full name (None: no package identity)
    pub(super) fn full_name() -> Option<String> {
        let mut n = 0u32;
        // SAFETY: a length query without a buffer, then the name into n wide chars.
        unsafe {
            if GetCurrentPackageFullName(&mut n, core::ptr::null_mut()) != ERROR_INSUFFICIENT_BUFFER || n == 0 || n > 1024 {
                return None;
            }
            let mut buf: Vec<u16> = alloc::vec![0; n as usize];
            if GetCurrentPackageFullName(&mut n, buf.as_mut_ptr()) != 0 {
                return None;
            }
            let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
            Some(String::from_utf16_lossy(&buf[..len]))
        }
    }

    fn hr(e: i32) -> String {
        format!("WinRT: 0x{:08X}", e as u32)
    }

    pub(super) fn updates() -> Result<MsixUpdates, String> {
        if full_name().is_none() {
            return Err(String::from("no MSIX package"));
        }
        win_winrt::mta(|| {
            let statics = win_winrt::factory("Windows.ApplicationModel.Package", &IID_IPACKAGESTATICS).map_err(hr)?;
            let package = statics.get_obj(PS_CURRENT).map_err(hr)?;
            // Windows 10 1809: IPackage6
            let p6 = package.query(&IID_IPACKAGE6).ok_or_else(|| String::from("Package.GetAppInstallerInfo needs Windows 10 1809"))?;
            let mut info = 0usize;
            // SAFETY: GetAppInstallerInfo(this, out IAppInstallerInfo) (null without one).
            let r = unsafe {
                let f: unsafe extern "system" fn(usize, *mut usize) -> i32 = core::mem::transmute(p6.vfn(P6_APPINSTALLERINFO));
                f(p6.ptr(), &mut info)
            };
            if r < 0 {
                return Err(hr(r));
            }
            let Some(info) = Obj::own(info) else { return Ok(MsixUpdates { appinstaller: String::new(), availability: 0 }) };
            let uri = info.get_obj(AI_URI).and_then(|u| u.get_text(URI_ABSOLUTE)).map_err(hr)?;
            let op = p6.get_obj(P6_CHECKUPDATE).map_err(hr)?;
            win_winrt::wait(&op, CHECK_MS).map_err(hr)?;
            let result = op.get_obj(OP_RESULTS).map_err(hr)?;
            let availability = result.get_i32(UR_AVAILABILITY).map_err(hr)?;
            Ok(MsixUpdates { appinstaller: uri, availability: availability.clamp(0, 4) as u32 })
        })
        .unwrap_or_else(|| Err(String::from("no thread for WinRT")))
    }
}
