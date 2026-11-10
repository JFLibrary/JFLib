// form -- the kind of device the app runs on: a desktop, a tablet or a phone
//
//   form_factor() -> FF_DESKTOP / FF_TABLET / FF_PHONE
//   mobile() -> bool                 a tablet or a phone (form_factor() != FF_DESKTOP)
//
// The platform decides, never the window's size alone: Windows, Linux, macOS and the browser are
// FF_DESKTOP however small a window is dragged (a narrow desktop window keeps the desktop's
// behaviour); Android and iOS are FF_TABLET, and FF_PHONE where the main window's shorter side is
// below 600 DIP (Android's own line between a phone and a tablet, `sw600dp`; a tablet's app in a
// narrow split screen is laid out like a phone's then).  The shorter side keeps a phone a phone
// when it is turned.
//
// jfgui_FORM=desktop|tablet|phone decides instead (any platform: tests, and a look at the phone's
// layout on the desktop; scripts: `system env jfgui_FORM phone`).
//
// What the library does by itself on a phone or a tablet (only there):
//   the title bar has no caption buttons (chrome.rs: the system owns the app's window);
//   a menu bar that does not fit its row becomes one menu button (popup.rs, menubar);
//   the status bar's items roll sideways with the finger when the bar is too narrow (chrome.rs);
//   windows and content dialogs keep inside the screen, what does not fit rolls (win.rs, chrome.rs);
//   Android and iOS paint the system's bars in the colours at the app's edges (the hosts).
// Apps ask form_factor() for what only they know: which panels a phone shows, which a tablet.

use super::consts::*;
use super::Ui;
#[cfg(target_os = "android")]
use gfx::Surface;

/// a tablet's shorter side is at least this many DIP (Android's sw600dp)
const TABLET_DIP: i32 = 600;

impl Ui {
    /// ui_form_factor: FF_DESKTOP, FF_TABLET or FF_PHONE (the platform's; jfgui_FORM decides
    /// instead).  A desktop window stays FF_DESKTOP however narrow it is.
    pub fn form_factor(&self) -> u32 {
        self.form
    }

    /// ui_mobile: the app runs on a phone or a tablet (form_factor() != FF_DESKTOP)
    pub fn mobile(&self) -> bool {
        self.form != FF_DESKTOP
    }

    // form_update(w, h): the main window's pass begins (w x h px): the form factor of this frame
    pub(crate) fn form_update(&mut self, w: i32, h: i32) {
        let side = self.px2dip(w.min(h));
        let mobile_os = cfg!(any(target_os = "android", target_os = "ios"));
        self.form = match plat::env("jfgui_FORM").as_deref() {
            Some("phone") => FF_PHONE,
            Some("tablet") => FF_TABLET,
            Some("desktop") => FF_DESKTOP,
            _ if !mobile_os => FF_DESKTOP,
            _ if side > 0 && side < TABLET_DIP => FF_PHONE,
            _ => FF_TABLET,
        };
    }
}

// (Android) edge_colors(fb) -> the colours (0xRRGGBB) along the frame's top, bottom, left and right
// edge: on each the most frequent of 9 samples (a widget touching the edge somewhere does not
// count); the host paints the system's bars in them
#[cfg(target_os = "android")]
pub(crate) fn edge_colors(fb: &Surface) -> [u32; 4] {
    let (w, h) = (fb.w, fb.h);
    if w <= 0 || h <= 0 || fb.px.len() < (fb.stride * (h - 1) + w * 4) as usize {
        return [0; 4];
    }
    let at = |x: i32, y: i32| {
        let o = (y * fb.stride + x * 4) as usize;
        (fb.px[o + 2] as u32) << 16 | (fb.px[o + 1] as u32) << 8 | fb.px[o] as u32
    };
    let edge = |f: &dyn Fn(i32) -> u32, len: i32| {
        let s: [u32; 9] = core::array::from_fn(|k| f(((2 * k as i32 + 1) * len / 18).min(len - 1)));
        let mut best = (s[0], 0);
        for &c in &s {
            let n = s.iter().filter(|&&d| d == c).count();
            if n > best.1 {
                best = (c, n);
            }
        }
        best.0
    };
    [
        edge(&|x| at(x, 0), w),
        edge(&|x| at(x, h - 1), w),
        edge(&|y| at(0, y), h),
        edge(&|y| at(w - 1, y), h),
    ]
}

