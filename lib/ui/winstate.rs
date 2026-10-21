// winstate -- the placement of the windows kept between runs (AF_SAVE_STATE)
//
//   state_save(&mut ini)    the main window (position, size, maximized) into [window], the
//                           tool windows (open, placement) into [window ID]
//   state_load(&ini)        read them back: the main window at once (window_place(0)), a
//                           tool window when window_begin meets its id next (its open flag
//                           and WinRect written into the app's Shared values)
//
// AF_SAVE_STATE in the AppDesc: the runtime reads config://windows.ini before it creates the
// main window (at the saved size and position, maximized when it was; --size wins) and
// writes it at the end of a window run, before UIC_EXIT.  jfgui_STATE=PATH uses another
// file, jfgui_STATE=0 none.  Headless runs (--shot, --script, agents) never touch the file;
// state_save / state_load work there as well (the main window: its size only).
//
// [window]: x / y (screen px of the client area's top left; Wayland places windows itself
// and ignores them), w / h (DIP, the size before maximizing or minimizing), max (0 / 1).  A
// position off every monitor (a monitor that is gone) is centred on the work area of the
// nearest one.  [window ID] (decimal id): open (0 / 1) and x / y / w / h (DIP, relative to
// the owner's client area: the WinRect).  Only tool windows (WF_TOOL, not WF_DIALOG /
// WF_MODAL) are kept; sections of windows that did not run this time stay in the file.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use base::Ini;

use super::consts::*;
use super::layout::px2dip_at;
use super::Ui;

const SEC_MAIN: &str = "window";
const SEC_PREFIX: &str = "window ";
const SIZE_MAX_DIP: i32 = 8192;
const POS_MAX: i32 = 1 << 20;
const VISIBLE_MIN: i32 = 48; // px of the window that must stay on a monitor

/// the main window's saved placement
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub(crate) struct MainState {
    pub(crate) x: i32,
    pub(crate) y: i32,
    pub(crate) pos: bool,
    pub(crate) w: i32,
    pub(crate) h: i32,
    pub(crate) max: bool,
}

/// a tool window's saved state not applied yet
#[derive(Clone, Copy, Debug)]
pub(crate) struct Saved {
    id: u32,
    open: i32, // -1: not saved
    rect: Option<WinRect>,
    open_done: bool, // the open flag went to the app (window_begin met the id)
}

/// window kinds whose state is kept: tool windows that are no dialogs
fn keepable(flags: u32) -> bool {
    flags & WF_TOOL != 0 && flags & (WF_DIALOG | WF_MODAL) == 0
}

fn int(ini: &Ini, sec: &str, key: &str) -> Option<i64> {
    let v = ini.get_int(sec, key, i64::MIN);
    (v != i64::MIN).then_some(v)
}

/// the main window's entry of a state file
pub(crate) fn main_saved(ini: &Ini) -> Option<MainState> {
    let w = int(ini, SEC_MAIN, "w")?;
    let h = int(ini, SEC_MAIN, "h")?;
    if !(1..=SIZE_MAX_DIP as i64).contains(&w) || !(1..=SIZE_MAX_DIP as i64).contains(&h) {
        return None;
    }
    let (x, y) = (int(ini, SEC_MAIN, "x"), int(ini, SEC_MAIN, "y"));
    let pos = matches!((x, y), (Some(x), Some(y)) if x.abs() < POS_MAX as i64 && y.abs() < POS_MAX as i64);
    Some(MainState {
        x: if pos { x.unwrap_or(0) as i32 } else { 0 },
        y: if pos { y.unwrap_or(0) as i32 } else { 0 },
        pos,
        w: w as i32,
        h: h as i32,
        max: int(ini, SEC_MAIN, "max").unwrap_or(0) != 0,
    })
}

/// the state file of a window run: jfgui_STATE (a path, 0 none, 1 the default) or, with
/// AF_SAVE_STATE, config://windows.ini
pub(crate) fn state_file(flags: u32) -> Option<String> {
    match plat::env("jfgui_STATE") {
        Some(v) if v == "0" => None,
        Some(v) if !v.is_empty() && v != "1" => Some(v),
        Some(_) => Some(String::from("config://windows.ini")),
        None => (flags & AF_SAVE_STATE != 0).then(|| String::from("config://windows.ini")),
    }
}

/// a client rectangle (px) with less than VISIBLE_MIN px of its top edge on a work area (or
/// its top above it: the title bar out of reach): centred on the work area instead
pub(crate) fn fit_area(x: i32, y: i32, w: i32, h: i32, area: [i32; 4]) -> (i32, i32) {
    let [ax, ay, aw, ah] = area;
    if aw <= 0 || ah <= 0 {
        return (x, y);
    }
    let seen_x = (x + w).min(ax + aw) - x.max(ax);
    if seen_x >= VISIBLE_MIN.min(w) && y >= ay && y <= ay + ah - VISIBLE_MIN {
        return (x, y);
    }
    (ax + ((aw - w) / 2).max(0), ay + ((ah - h) / 2).max(VISIBLE_MIN.min(ah / 8)))
}

impl Ui {
    /// write the placement of the main window and of the tool windows into ini (see the
    /// module header)
    pub fn state_save(&mut self, ini: &mut Ini) {
        if let Some(m) = self.ws_main_now() {
            if m.pos {
                ini.set_int(SEC_MAIN, "x", m.x as i64);
                ini.set_int(SEC_MAIN, "y", m.y as i64);
            } else {
                ini.remove(SEC_MAIN, "x");
                ini.remove(SEC_MAIN, "y");
            }
            ini.set_int(SEC_MAIN, "w", m.w as i64);
            ini.set_int(SEC_MAIN, "h", m.h as i64);
            ini.set_int(SEC_MAIN, "max", m.max as i64);
        }
        for i in 1..WIN_MAX {
            let r = &self.m.win.recs[i];
            if r.id == 0 || !keepable(r.flags) || r.id == super::inspect::INSPECT_WIN {
                continue; // (the inspector's window is no window of the app)
            }
            let sec = format!("{}{}", SEC_PREFIX, r.id);
            ini.set_int(&sec, "open", (r.state & super::win::UWI_OPEN != 0) as i64);
            if r.state & super::win::UWI_SHOWN != 0 {
                let rc = r.rect.get();
                ini.set_int(&sec, "x", rc.x as i64);
                ini.set_int(&sec, "y", rc.y as i64);
                ini.set_int(&sec, "w", rc.w as i64);
                ini.set_int(&sec, "h", rc.h as i64);
            }
        }
    }

    /// read the placements of state_save back (the main window now, the tool windows when
    /// window_begin meets them)
    pub fn state_load(&mut self, ini: &Ini) {
        if let Some(m) = main_saved(ini) {
            let mut flags = 0;
            if m.pos && !self.headless {
                flags |= WRF_POS | WRF_ABS;
            }
            if m.max && !self.headless {
                flags |= WRF_MAX;
            }
            self.window_place(0, &WinRect::new(m.x, m.y, m.w, m.h, flags));
            self.m.win.norm = m;
        }
        self.ws_windows(ini);
    }

    /// (runtime) the tool windows of a state file, applied when window_begin meets them
    pub(crate) fn ws_windows(&mut self, ini: &Ini) {
        let mut saved = Vec::new();
        for sec in ini.sections() {
            let Some(id) = sec.strip_prefix(SEC_PREFIX).and_then(|n| n.trim().parse::<u32>().ok()) else { continue };
            if id == 0 {
                continue;
            }
            let open = int(ini, sec, "open").map_or(-1, |v| (v != 0) as i32);
            let rect = match (int(ini, sec, "x"), int(ini, sec, "y"), int(ini, sec, "w"), int(ini, sec, "h")) {
                (Some(x), Some(y), Some(w), Some(h)) if x.abs() < POS_MAX as i64 && y.abs() < POS_MAX as i64 => Some(WinRect::new(
                    x as i32,
                    y as i32,
                    w.clamp(1, SIZE_MAX_DIP as i64) as i32,
                    h.clamp(1, SIZE_MAX_DIP as i64) as i32,
                    WRF_POS,
                )),
                _ => None,
            };
            saved.retain(|s: &Saved| s.id != id);
            saved.push(Saved { id, open, rect, open_done: false });
        }
        self.m.win.saved = saved;
    }

    /// (window_begin, before the window is looked at) a saved open flag goes into the app's
    /// Shared once; windows that are not kept forget their entry
    pub(crate) fn ws_open(&mut self, id: u32, flags: u32, open: Option<&super::Shared<i32>>) {
        let Some(k) = self.m.win.saved.iter().position(|s| s.id == id) else { return };
        if !keepable(flags) {
            self.m.win.saved.remove(k);
            return;
        }
        let s = &mut self.m.win.saved[k];
        if s.open_done {
            return;
        }
        s.open_done = true;
        if let (Some(o), true) = (open, s.open >= 0) {
            o.set(s.open);
        }
    }

    /// (window_begin, the record's WinRect set) a saved placement goes into it once
    pub(crate) fn ws_rect(&mut self, i: u32, id: u32) {
        let Some(k) = self.m.win.saved.iter().position(|s| s.id == id) else { return };
        let s = self.m.win.saved.remove(k);
        let Some(src) = s.rect else { return };
        let r = &mut self.m.win.recs[i as usize & (WIN_MAX - 1)];
        let rc = r.rect.get();
        r.rect.set(WinRect { x: src.x, y: src.y, w: src.w, h: src.h, flags: (rc.flags & !(WRF_CHANGED | WRF_MAX)) | WRF_POS });
        r.state &= !super::win::UWI_DEFPOS;
    }

    /// (window events of the main window) remember its placement while it is neither
    /// maximized nor minimized: the size and position state_save keeps for those.  A new
    /// placement is a candidate first: X11 reports the size of a maximize before the state,
    /// so a candidate as large as the window that turns out maximized is dropped
    pub(crate) fn ws_track(&mut self) {
        if self.headless {
            return;
        }
        let big = self.ws_big();
        self.ws_commit(big);
        if !big {
            self.m.win.cand = self.ws_main_live();
        }
    }

    // maximized, minimized or full screen
    fn ws_big(&self) -> bool {
        self.uiw_plat_state(0) & (plat::WST_MAXIMIZED | plat::WST_MINIMIZED | plat::WST_FULLSCREEN) != 0
    }

    // the candidate becomes the normal placement unless the window is big now at its size
    fn ws_commit(&mut self, big: bool) {
        if let Some(c) = self.m.win.cand.take() {
            if !big || self.ws_main_live().is_none_or(|l| (l.w, l.h) != (c.w, c.h)) {
                self.m.win.norm = c;
            }
        }
    }

    // the main window as it is (position when known, size in DIP; max false)
    fn ws_main_live(&self) -> Option<MainState> {
        let fb = self.win_fb(0);
        let dpi = self.win_dpi(0);
        if fb.w <= 0 || fb.h <= 0 {
            return None;
        }
        // the platform's client origin (None: not known, Wayland), else the last E_WINPOS,
        // else the last known one
        let rc = self.m.win.recs[0].rect.get();
        let at = if self.headless {
            None
        } else if let Some(r) = plat::win_get_rect(self.uiw_pw(0)) {
            Some((r[0], r[1]))
        } else if rc.flags & WRF_POS != 0 {
            Some((rc.x, rc.y))
        } else if self.m.win.norm.pos {
            Some((self.m.win.norm.x, self.m.win.norm.y))
        } else {
            None
        };
        Some(MainState {
            x: at.map_or(0, |a| a.0),
            y: at.map_or(0, |a| a.1),
            pos: at.is_some(),
            w: px2dip_at(fb.w, dpi),
            h: px2dip_at(fb.h, dpi),
            max: false,
        })
    }

    // what state_save writes for the main window: its placement when it is normal, else the
    // last normal one with the maximized flag
    fn ws_main_now(&mut self) -> Option<MainState> {
        if self.headless {
            return self.ws_main_live();
        }
        let big = self.ws_big();
        self.ws_commit(big);
        let max = self.uiw_plat_state(0) & plat::WST_MAXIMIZED != 0;
        let mut m = if big { self.m.win.norm } else { self.ws_main_live().unwrap_or(self.m.win.norm) };
        if m.w <= 0 || m.h <= 0 {
            return None;
        }
        m.max = max;
        Some(m)
    }
}
