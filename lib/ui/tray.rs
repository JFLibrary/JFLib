// tray -- the app's symbol in the notification area and system notifications (plat::tray)
//
//   tray_set(icon, tooltip, menu) -> bool   show or update the symbol: icon None = the
//                                    program's own; menu = Mi rows like a window menu (labels
//                                    from the command table, CF_DISABLED / CF_CHECKED /
//                                    CF_HIDDEN of cmd_state at this moment: call it again after
//                                    a change); false: no notification area here
//   tray_remove()
//   notify(title, body, icon) -> id  a system notification (0: not shown)
//   notify_close(id)                 tray_supported() -> bool
//   bring_to_front()                 restore and activate the main window (e.g. on UIC_TRAY)
// Headless runs (scripts, --shot) never show anything: plat::tray's dry run prints it.
// Events (E_WAKE): a chosen menu row runs App::command(id, 0) like a row of the window's menu;
//   a click on the symbol App::command(UIC_TRAY, TRAY_ACTIVATE), a double / middle click
//   UIC_TRAY TRAY_SECONDARY; a clicked notification UIC_NOTE (arg = its id), one that went
//   away unclicked UIC_NOTE_GONE (arg = its id).

use alloc::string::String;
use alloc::vec::Vec;

use gfx::{Surface, FMT_PARGB, FMT_XRGB};
use plat::tray::{self, Note, Tray, TrayEvent, TrayIcon, TrayItem, TI_CHECKED, TI_DISABLED, TI_SEPARATOR};

use super::consts::*;
use super::popup::Mi;
use super::Ui;

// straight ARGB pixels of a surface (at most 256 x 256; also the taskbar's badge, taskbar.rs)
pub(super) fn icon_of(sf: &Surface) -> Option<TrayIcon> {
    if sf.w <= 0 || sf.h <= 0 || sf.w > 256 || sf.h > 256 {
        return None;
    }
    let mut px = Vec::with_capacity((sf.w * sf.h) as usize);
    for y in 0..sf.h {
        for x in 0..sf.w {
            let o = (y * sf.stride + x * 4) as usize;
            let v = u32::from_le_bytes(sf.px.get(o..o + 4)?.try_into().ok()?);
            px.push(match sf.fmt {
                FMT_XRGB => v | 0xFF00_0000,
                FMT_PARGB => {
                    let a = v >> 24;
                    if a == 0 {
                        0
                    } else {
                        let un = |c: u32| ((c * 255 + a / 2) / a).min(255);
                        (a << 24) | (un((v >> 16) & 255) << 16) | (un((v >> 8) & 255) << 8) | un(v & 255)
                    }
                }
                _ => v,
            });
        }
    }
    Some(TrayIcon { w: sf.w as u32, h: sf.h as u32, px })
}

impl Ui {
    // the menu rows of Mi records (the commands' state now)
    fn tray_items(&self, menu: &[Mi]) -> Vec<TrayItem> {
        let mut out = Vec::new();
        for m in menu {
            match *m {
                Mi::Cmd(c) => {
                    let f = self.cmd_flags(c);
                    if f & CF_HIDDEN != 0 {
                        continue;
                    }
                    let label = self.cmd_find(c).map_or(String::new(), |r| String::from(r.label));
                    let flags = if f & CF_DISABLED != 0 { TI_DISABLED } else { 0 } | if f & CF_CHECKED != 0 { TI_CHECKED } else { 0 };
                    out.push(TrayItem { id: c, label, flags, sub: Vec::new() });
                }
                Mi::Sep => out.push(TrayItem { flags: TI_SEPARATOR, ..TrayItem::default() }),
                Mi::Sub(label, items) | Mi::Menu(label, items) => {
                    out.push(TrayItem { id: 0, label: String::from(label), flags: 0, sub: self.tray_items(items) })
                }
                Mi::Item(id, label, _) => out.push(TrayItem { id, label: String::from(label), flags: 0, sub: Vec::new() }),
                Mi::List(base, max, placeholder) => {
                    let rows: Vec<String> = self.m.popup.lists.iter().find(|e| e.0 == base).map(|e| e.1.clone()).unwrap_or_default();
                    if rows.is_empty() && !placeholder.is_empty() {
                        out.push(TrayItem { id: 0, label: String::from(placeholder), flags: TI_DISABLED, sub: Vec::new() });
                    }
                    for (k, r) in rows.iter().take(max as usize).enumerate() {
                        out.push(TrayItem { id: base + k as u32, label: r.replace('&', "&&"), flags: 0, sub: Vec::new() });
                    }
                }
            }
        }
        out
    }

    /// show or update the symbol in the notification area -> false: there is none here
    pub fn tray_set(&mut self, icon: Option<&Surface>, tooltip: &str, menu: &[Mi]) -> bool {
        tray::set_dry_run(self.headless);
        let t = Tray { icon: icon.and_then(icon_of), tooltip: String::from(tooltip), menu: self.tray_items(menu) };
        tray::tray_set(&t)
    }

    /// take the symbol away
    pub fn tray_remove(&mut self) {
        tray::tray_remove();
    }

    /// a system notification -> its id (0: not shown); UIC_NOTE / UIC_NOTE_GONE report it
    pub fn notify(&mut self, title: &str, body: &str, icon: Option<&Surface>) -> u32 {
        tray::set_dry_run(self.headless);
        tray::notify(&Note { title: String::from(title), body: String::from(body), icon: icon.and_then(icon_of) })
    }

    /// take a notification away
    pub fn notify_close(&mut self, id: u32) {
        tray::notify_close(id);
    }

    /// whether this system shows the symbol (a notification area; GNOME: only with the
    /// AppIndicator extension); notifications work without one
    pub fn tray_supported(&self) -> bool {
        tray::tray_supported()
    }

    /// restore the main window when it is minimised and activate it
    pub fn bring_to_front(&mut self) {
        if self.headless {
            return;
        }
        if plat::win_state(0) & plat::WST_MINIMIZED != 0 {
            plat::win_cmd(0, plat::WCMD_RESTORE);
        }
        plat::win_activate(0);
    }

    /// E_WAKE: the symbol's and the notifications' events -> App::command
    pub(crate) fn tray_poll(&mut self) {
        for e in tray::tray_take() {
            match e {
                TrayEvent::Activate => self.cmd_run(UIC_TRAY, TRAY_ACTIVATE),
                TrayEvent::SecondaryActivate => self.cmd_run(UIC_TRAY, TRAY_SECONDARY),
                TrayEvent::Menu(id) => self.cmd_run(id, 0),
                TrayEvent::NoteClicked(id) => self.cmd_run(UIC_NOTE, id),
                TrayEvent::NoteClosed(id) => self.cmd_run(UIC_NOTE_GONE, id),
            }
        }
    }
}
