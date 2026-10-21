// icc -- the colour profile of the monitor a window is on
//          [src/plat/win/win_icc.s, src/plat/linux/lx_icc.s; docs/modules/COLOR_MANAGEMENT.md]
//
//   monitor_icc(w) -> profile bytes / None
//        Windows: the ICC profile of the monitor that shows window w (0 = the main window;
//        other windows use the main window's monitor; ICC_MON_PRIMARY (-1) = the primary
//        monitor, no window needed): GetMonitorInfoW -> CreateDCW(device) -> GetICMProfileW
//        -> the file.  No window (headless) -> None.
//        Linux X11: the root window property _ICC_PROFILE (ICC Profiles in X Specification;
//        set by colord / xiccd / dispwin for screen 0); profiles above 1 MB are ignored.
//        Wayland: None -- the core protocol has no colour management (the compositor shows
//        sRGB), so callers assume sRGB.  No display (headless): None.
//        Every profile is validated (icc_check: header, 'acsp', version 2..5, tag table).
//   monitor_key(w) -> u64   Windows: the HMONITOR of that monitor (0 = none) -- cheap; poll
//        it every frame and query the profile again when it changes (window moved to another
//        monitor).  Linux: 0 (the X11 property is per screen, not per monitor).
//
// icc_check is a private copy of img_core::icc::icc_check (plat does not depend on the image
// crates).  Linux: the X11 requests go through linux_x11.rs (x11s::root_property, marked
// plat-shared there).

use alloc::vec::Vec;

/// monitor_icc / monitor_key: the primary monitor instead of a window's
pub const ICC_MON_PRIMARY: i32 = -1;

fn be32(d: &[u8], o: usize) -> u32 {
    u32::from_be_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]])
}

/// a usable profile: icc_check > 0 (declared size 132..len, 'acsp', major version 2..5,
/// every tag inside the declared size)  [icc_check of src/img/icc.s]
fn icc_ok(data: &[u8]) -> bool {
    let len = data.len() as u64;
    if len < 132 {
        return false;
    }
    let size = be32(data, 0) as u64;
    if size < 132 || size > len || &data[36..40] != b"acsp" || (data[8] as u32).wrapping_sub(2) > 3 {
        return false;
    }
    let n = be32(data, 128) as u64;
    if n > (size - 132) / 12 {
        return false;
    }
    (0..n as usize).all(|t| {
        let off = be32(data, 136 + t * 12) as u64;
        let sz = be32(data, 140 + t * 12) as u64;
        off <= size && size - off >= sz
    })
}

// ---------------------------------------------------------------- Windows  [win_icc.s]

#[cfg(windows)]
mod os {
    use alloc::vec::Vec;

    const MONITOR_NEAREST: u32 = 2;
    const MON_PRIMARY_FLAG: u32 = 1; // MONITOR_DEFAULTTOPRIMARY
    const MIX_SIZE: usize = 104; // MONITORINFOEXW
    const MIX_DEVICE: usize = 40; // WCHAR szDevice[32]
    const WPATH_CAP: usize = 520; // WCHARs for GetICMProfileW

    #[allow(clashing_extern_declarations)] // (win_api.rs: with its own MONITORINFO type)
    #[link(name = "user32", kind = "raw-dylib")]
    unsafe extern "system" {
        fn MonitorFromWindow(hwnd: isize, flags: u32) -> isize;
        fn GetMonitorInfoW(mon: isize, mi: *mut u8) -> i32;
    }

    #[link(name = "gdi32", kind = "raw-dylib")]
    unsafe extern "system" {
        fn CreateDCW(driver: *const u16, device: *const u16, port: *const u16, dm: *const u8) -> isize;
        fn DeleteDC(hdc: isize) -> i32;
        fn GetICMProfileW(hdc: isize, size: *mut u32, name: *mut u16) -> i32;
    }

    /// the HMONITOR of window w's monitor (the main window's; -1: the primary) | 0
    /// [plat_monitor_key]
    pub fn monitor_key(w: i32) -> u64 {
        // SAFETY: MonitorFromWindow takes any HWND (0 with DEFAULTTOPRIMARY = the primary).
        unsafe {
            if w == super::ICC_MON_PRIMARY {
                return MonitorFromWindow(0, MON_PRIMARY_FLAG) as u64;
            }
            let h = crate::win::hwnd_main();
            if h == 0 {
                return 0;
            }
            MonitorFromWindow(h, MONITOR_NEAREST) as u64
        }
    }

    /// see the module header  [plat_monitor_icc]
    pub fn monitor_icc(w: i32) -> Option<Vec<u8>> {
        let mon = monitor_key(w) as isize;
        if mon == 0 {
            return None;
        }
        let mut mi = [0u8; MIX_SIZE];
        mi[..4].copy_from_slice(&(MIX_SIZE as u32).to_le_bytes());
        // SAFETY: a MONITORINFOEXW with its size set.
        if unsafe { GetMonitorInfoW(mon, mi.as_mut_ptr()) } == 0 {
            return None;
        }
        let mut dev = [0u16; 32];
        for (k, d) in dev.iter_mut().enumerate() {
            *d = u16::from_le_bytes([mi[MIX_DEVICE + k * 2], mi[MIX_DEVICE + k * 2 + 1]]);
        }
        dev[31] = 0;
        // SAFETY: the NUL-terminated device name as driver and device.
        let hdc = unsafe { CreateDCW(dev.as_ptr(), dev.as_ptr(), core::ptr::null(), core::ptr::null()) };
        if hdc == 0 {
            return None;
        }
        let mut path = [0u16; WPATH_CAP];
        let mut n = WPATH_CAP as u32;
        // SAFETY: a buffer of n wide chars; the DC is deleted right after.
        let ok = unsafe {
            let ok = GetICMProfileW(hdc, &mut n, path.as_mut_ptr());
            DeleteDC(hdc);
            ok
        };
        if ok == 0 {
            return None;
        }
        path[WPATH_CAP - 1] = 0;
        let len = path.iter().position(|&c| c == 0).unwrap_or(0);
        let p = crate::from_wide(&path[..len]);
        if p.is_empty() {
            return None;
        }
        let data = crate::read_file_raw(&p)?;
        if super::icc_ok(&data) { Some(data) } else { None }
    }
}

// ---------------------------------------------------------------- Linux  [lx_icc.s]

#[cfg(target_os = "linux")]
mod os {
    use alloc::vec::Vec;
    use core::sync::atomic::{AtomicU32, Ordering};

    /// the atom of _ICC_PROFILE: 0 not asked, u32::MAX none  (g_icc_atom)
    static ATOM: AtomicU32 = AtomicU32::new(0);

    /// 0 (the property is per screen)  [plat_monitor_key]
    pub fn monitor_key(_w: i32) -> u64 {
        0
    }

    /// see the module header  [plat_monitor_icc]
    pub fn monitor_icc(_w: i32) -> Option<Vec<u8>> {
        if crate::disp_backend() != crate::BK_X11 {
            return None;
        }
        let mut atom = ATOM.load(Ordering::Relaxed);
        let r = crate::x11s::root_property(b"_ICC_PROFILE", &mut atom);
        ATOM.store(atom, Ordering::Relaxed);
        let r = r?;
        // format 8, nothing after this chunk, at least the header
        if r.len() < 32 || r[1] != 8 || u32::from_le_bytes([r[12], r[13], r[14], r[15]]) != 0 {
            return None;
        }
        let n = u32::from_le_bytes([r[16], r[17], r[18], r[19]]) as usize;
        if n < 132 || 32 + n > r.len() {
            return None;
        }
        let data = r[32..32 + n].to_vec();
        if super::icc_ok(&data) { Some(data) } else { None }
    }
}

/// the ICC profile of the monitor showing window w (validated), None = assume sRGB
/// [plat_monitor_icc]
pub fn monitor_icc(w: i32) -> Option<Vec<u8>> {
    os::monitor_icc(w)
}

/// a cheap key of the monitor showing window w (poll it every frame; 0 = unknown)
/// [plat_monitor_key]
pub fn monitor_key(w: i32) -> u64 {
    os::monitor_key(w)
}
