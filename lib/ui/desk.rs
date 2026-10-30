// desk -- windows inside an area of a window: the app's programs in windows of their own
// (title bar with icon, title and buttons, moving, resizing at every edge and corner,
// maximising, minimising, z-order, snapping, keyboard, screen readers) and a taskbar; the
// state is the app's DeskState
//
//   desk_begin(id, &mut st, wins)     the area takes the next place (without next_size it fills
//                                     the box); wins describe every open window each frame, in
//                                     the order of the taskbar (DeskWin: key != 0, title, icon,
//                                     DWF_* flags, minimum and first size)
//   desk_next(&mut st) -> Some(key)   the next window to fill, from the bottom of the z-order to
//                                     the top (minimised ones are left out): emit its content (a
//                                     vertical box that clips) and call it again until None
//   desk_end(&mut st) -> Vec<DeskEv>  what the user did: close requests (the app takes the
//                                     window out of its list, or asks first), activation, moves,
//                                     resizes, state changes (also those of the taskbar)
//   desk_taskbar_begin(id)            a taskbar of desk `id`: a row with the style's surface;
//   desk_taskbar_end()                the app's own items (a start button, a clock) go before
//                                     and after desk_taskbar.  Between desk_begin and desk_end
//                                     without next_size / next_rect it takes the strip the
//                                     area keeps free (DeskState::set_reserve)
//   desk_taskbar(id, &mut st, flags)  a button per window of desk `id` (icon, title; the
//                                     active one pressed): a click activates, a second click
//                                     minimises, a right click (menu key, Shift+F10) opens
//                                     Wiederherstellen / Minimieren / Maximieren / Schließen;
//                                     what does not fit goes into a "Weitere Fenster" menu at
//                                     the end; DTB_ICONS: icons only
//   DeskState                         plain data: the windows' places, the z-order, maximised /
//                                     minimised / snapped, the active one -- save() -> text,
//                                     load(text), activate / minimize / maximize / restore /
//                                     toggle_max / snap / cycle / set_rect / forget, order /
//                                     active / state / rect / contains, set_reserve
//
// The windows: a press anywhere on one brings it to the front and makes it active (drawn as
// active, its content gets the keys).  The title bar moves it (a maximised or snapped one comes
// back to its size under the pointer); while it is dragged its edges stick to the area's edges,
// and with the pointer at the area's left or right edge it snaps to that half, at the top edge
// it maximises (a preview shows where).  Every edge and corner resizes it down to its minimum
// size; a double click on the title bar maximises or restores it; the icon and a right click on
// the title bar open its menu.  Esc cancels a drag.  The area keeps every window reachable: when it shrinks, at least
// KEEP DIP of a window and its whole title bar stay inside (shown there, the place in the state
// stays).  Maximised windows fill the area minus the reserved strip (a taskbar inside it); new
// windows are cascaded from the top left.  Places are logical (DIP from the area's start
// corner): a mirrored (RTL) window shows them mirrored, the title bar's buttons at the left.
// Keyboard (while the focus is inside the area, or nowhere): Alt+F6 / Alt+Shift+F6 the next /
// previous window (the active one goes to the back), Alt+F10 maximises / restores the active
// one, Alt+F9 minimises it, Alt+- opens its menu.  Tab never enters the windows behind the
// active one (their widgets are no Tab stops); each window is an F6 pane, and a focus that
// arrives in a window behind (F6, a screen reader, focus(id)) brings it to the front.  The
// active window gets the keyboard focus back where it was (else its first Tab stop).
// Screen readers: each shown window is a dialog record named by its title (" (maximiert)"
// after it when it is), the active one selected; its title bar, buttons and content are inside
// it, whatever the windows above cover.  The taskbar is a group "Taskleiste" with a button per
// window (" (minimiert)" after a minimised one's title, the active one pressed).
// Structural changes (buttons, menus, keys) are applied at desk_end, followed by another pass;
// drags change the place at once.  Widget ids are parts of the area id (pid).

use alloc::string::String;
use alloc::vec::Vec;

use super::consts::*;
use super::keys::*;
use super::{Sp, Ui};

/// DeskWin flags: no close button (the app closes the window itself)
pub const DWF_NOCLOSE: u32 = 1;
/// no minimise button (and no minimising by keys or the taskbar)
pub const DWF_NOMIN: u32 = 2;
/// no maximise button (no double click, no snapping to the top edge)
pub const DWF_NOMAX: u32 = 4;
/// a fixed size: no resize grips, no maximising, no snapping to a half
pub const DWF_FIXED: u32 = 8;
/// a new window starts maximised (an IDE)
pub const DWF_MAXIMIZED: u32 = 16;
/// no taskbar button
pub const DWF_NOTASK: u32 = 32;
/// DeskState::state: maximised (with DW_MIN: it comes back maximised)
pub const DW_MAX: u32 = 1;
/// minimised (hidden until restored)
pub const DW_MIN: u32 = 2;
/// snapped to the start (left) / end (right) half of the area (mirrored windows show it the
/// other way round)
pub const DW_LEFT: u32 = 4;
pub const DW_RIGHT: u32 = 8;
/// desk_taskbar flags: icons only
pub const DTB_ICONS: u32 = 1;

// metrics (DIP)
const DEF_W: i32 = 400; // a new window without DeskWin::size
const DEF_H: i32 = 280;
const MIN_W: i32 = 160; // the smallest window without DeskWin::min
const MIN_H: i32 = 96;
const CASCADE: i32 = 28;
const KEEP: i32 = 64; // of a window's width that stays inside the area
const MAGNET: i32 = 10; // window edges this close to the area's edges stick to them
const ZONE: i32 = 4; // the pointer this close to the area's edge (or beyond) snaps
const GRIP_OUT: i32 = 4; // resize bands: outside the frame
const GRIP_IN: i32 = 2; // ... and inside it
const SIZE_MAX: i32 = 16384;
const TASK_H: i32 = 40; // a taskbar without a size
const TASK_MIN: i32 = 96; // the narrowest taskbar button (with a title)
const TASK_MAX: i32 = 200;
const TASK_GAP: i32 = 2;
const OVER_W: i32 = 32; // the "Weitere Fenster" button

// edges of a resize
const E_LEFT: u32 = 1;
const E_TOP: u32 = 2;
const E_RIGHT: u32 = 4;
const E_BOTTOM: u32 = 8;
// the eight grips: edges, cursor (left to right)
const GRIPS: [(u32, u32); 8] = [
    (E_TOP, CUR_SIZE_NS),
    (E_BOTTOM, CUR_SIZE_NS),
    (E_LEFT, CUR_SIZE_WE),
    (E_RIGHT, CUR_SIZE_WE),
    (E_TOP | E_LEFT, CUR_SIZE_NWSE),
    (E_TOP | E_RIGHT, CUR_SIZE_NESW),
    (E_BOTTOM | E_LEFT, CUR_SIZE_NESW),
    (E_BOTTOM | E_RIGHT, CUR_SIZE_NWSE),
];

// id parts (pid)
const P_AREA: u32 = 1;
const P_WIN: u32 = 2;
const P_BOX: u32 = 3;
const P_TITLE: u32 = 4;
const P_ICON: u32 = 5;
const P_MIN: u32 = 6;
const P_MAX: u32 = 7;
const P_CLOSE: u32 = 8;
const P_CONTENT: u32 = 9;
const P_PANE: u32 = 10;
const P_MENU: u32 = 11;
const P_BAR: u32 = 12;
const P_TASK: u32 = 13;
const P_TBTN: u32 = 14;
const P_TMENU: u32 = 15;
const P_OVER: u32 = 16;
const P_OITEM: u32 = 17;
const P_GRIP: u32 = 20; // .. + 7
const P_MITEM: u32 = 32; // .. + 3: the window menu's rows
const P_TITEM: u32 = 40; // .. + 3: the taskbar menu's rows

// a widget id of desk `desk`: part `part` of window `a`
fn pid(desk: u32, part: u32, a: u32) -> u32 {
    let mut h = desk.wrapping_mul(0x9E37_79B1) ^ part.wrapping_mul(0x85EB_CA6B) ^ a.wrapping_mul(0xC2B2_AE35) ^ 0x5D35_E7A1;
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^= h >> 12;
    h | 0x8000_0000
}

fn inside(r: [i32; 4], p: [i32; 2]) -> bool {
    p[0] >= r[0] && p[0] < r[0] + r[2] && p[1] >= r[1] && p[1] < r[1] + r[3]
}

/// a window as the app describes it to desk_begin every frame
#[derive(Clone, Copy, Debug)]
pub struct DeskWin<'a> {
    /// != 0 and stable (the saved state refers to it)
    pub key: u32,
    /// shown in its title bar, its taskbar button and to screen readers
    pub title: &'a str,
    /// IC_* of its title bar and taskbar button (0 = none)
    pub icon: u32,
    /// DWF_*
    pub flags: u32,
    /// the smallest content size (DIP, without the title bar; 0 = the library's)
    pub min: [i32; 2],
    /// the size a new window gets (DIP, without the title bar; 0 = the library's)
    pub size: [i32; 2],
}

impl<'a> DeskWin<'a> {
    /// a window with the library's sizes and every button
    pub const fn new(key: u32, title: &'a str, icon: u32) -> DeskWin<'a> {
        DeskWin { key, title, icon, flags: 0, min: [0, 0], size: [0, 0] }
    }
    /// add DWF_* flags
    pub const fn flags(mut self, f: u32) -> DeskWin<'a> {
        self.flags |= f;
        self
    }
    /// the smallest content size (DIP)
    pub const fn min(mut self, w: i32, h: i32) -> DeskWin<'a> {
        self.min = [w, h];
        self
    }
    /// the content size of a new window (DIP); DWF_FIXED windows keep it
    pub const fn size(mut self, w: i32, h: i32) -> DeskWin<'a> {
        self.size = [w, h];
        self
    }
}

/// what desk_end reports
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeskEv {
    /// the user wants to close the window (close button, menu, taskbar): take it out of the
    /// list of windows (DeskState keeps its place until forget), or ask first
    Close(u32),
    /// it became the active window
    Activate(u32),
    /// the user moved it (a drag ended)
    Move(u32),
    /// the user resized it
    Resize(u32),
    /// minimised, maximised, restored or snapped (DeskState::state)
    State(u32),
}

// ------------------------------------------------------------------ the state (DeskState)

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
struct Win {
    key: u32,
    /// the normal place: x, y from the area's start corner, outer width and height (DIP)
    rect: [i32; 4],
    max: bool,
    min: bool,
    /// 0 none, 1 the start half, 2 the end half
    snap: u8,
}

impl Win {
    fn state(&self) -> u32 {
        let mut s = 0;
        if self.max {
            s |= DW_MAX;
        }
        if self.min {
            s |= DW_MIN;
        }
        match self.snap {
            1 => s |= DW_LEFT,
            2 => s |= DW_RIGHT,
            _ => {}
        }
        s
    }
}

/// the windows of an area: kept by the app and passed to desk_begin / desk_next / desk_end /
/// desk_taskbar.  Plain data: save() / load() turn it into a few lines of text (`desk 1`, then
/// `win KEY X Y W H STATE` from the bottom of the z-order to the top -- x, y from the area's
/// start corner, outer size, all DIP; STATE the DW_* bits -- and `active KEY`; unknown lines
/// are skipped).  The reserved strip is no part of it.
#[derive(Clone, Default, Debug, PartialEq, Eq)]
pub struct DeskState {
    /// from the bottom of the z-order to the top
    wins: Vec<Win>,
    active: u32,
    /// new windows placed so far (the cascade)
    cascade: i32,
    /// the strip the area keeps free: start, top, end, bottom (DIP; not saved)
    reserve: [i32; 4],
}

impl DeskState {
    /// no windows: every window goes cascaded to the front in its first frame
    pub fn new() -> DeskState {
        DeskState::default()
    }

    fn find(&self, key: u32) -> Option<usize> {
        self.wins.iter().position(|w| w.key == key)
    }

    // windows the state does not know yet: cascaded (or maximised), on top, active
    fn adopt(&mut self, wins: &[DeskWin], area: [i32; 2], tbh: i32) {
        for d in wins {
            if d.key == 0 || self.find(d.key).is_some() {
                continue;
            }
            let (mw, mh) = min_size(d, tbh);
            let w = (if d.size[0] > 0 { d.size[0] } else { DEF_W }).max(mw);
            let h = (if d.size[1] > 0 { d.size[1] + tbh } else { DEF_H }).max(mh);
            // the next step of the cascade; from the start again where it would leave the area
            // (not before the area has its size)
            let mut k = self.cascade;
            let o = CASCADE / 2 + k * CASCADE;
            if k > 0 && area[0] > 0 && area[1] > 0 && (o + w > area[0] || o + h > area[1]) {
                k = 0;
                self.cascade = 0;
            }
            self.cascade += 1;
            let x = CASCADE / 2 + k * CASCADE;
            let y = CASCADE / 2 + k * CASCADE;
            let max = d.flags & DWF_MAXIMIZED != 0 && d.flags & (DWF_NOMAX | DWF_FIXED) == 0;
            self.wins.push(Win { key: d.key, rect: [x, y, w, h], max, min: false, snap: 0 });
            self.active = d.key;
        }
    }

    /// forget every window: each goes to its first place in the next frame
    pub fn reset(&mut self) {
        let reserve = self.reserve;
        *self = DeskState { reserve, ..DeskState::default() };
    }

    /// the strip the area keeps free for a taskbar inside it (DIP: start, top, end, bottom):
    /// maximised and snapped windows leave it out, the title bars stay above it; not saved
    pub fn set_reserve(&mut self, insets: [i32; 4]) {
        self.reserve = insets.map(|v| v.clamp(0, SIZE_MAX));
    }

    /// the reserved strip (set_reserve)
    pub fn reserve(&self) -> [i32; 4] {
        self.reserve
    }

    /// the state knows the window
    pub fn contains(&self, key: u32) -> bool {
        self.find(key).is_some()
    }

    /// the windows from the bottom of the z-order to the top (minimised ones too)
    pub fn order(&self) -> Vec<u32> {
        self.wins.iter().map(|w| w.key).collect()
    }

    /// the active window (0: none)
    pub fn active(&self) -> u32 {
        self.active
    }

    /// DW_* bits of the window (0: a normal window, or an unknown one)
    pub fn state(&self, key: u32) -> u32 {
        self.find(key).map_or(0, |i| self.wins[i].state())
    }

    /// the normal place of the window: x, y from the area's start corner, outer width and
    /// height (DIP) -- what it comes back to from maximised, snapped or minimised
    pub fn rect(&self, key: u32) -> Option<[i32; 4]> {
        self.find(key).map(|i| self.wins[i].rect)
    }

    /// set the normal place of a window (DIP; see rect); a maximised or snapped one shows it
    /// once restored
    pub fn set_rect(&mut self, key: u32, r: [i32; 4]) {
        if let Some(i) = self.find(key) {
            self.wins[i].rect = [r[0].clamp(-SIZE_MAX, SIZE_MAX), r[1].clamp(-SIZE_MAX, SIZE_MAX), r[2].clamp(1, SIZE_MAX), r[3].clamp(1, SIZE_MAX)];
        }
    }

    /// bring the window to the front and make it active (a minimised one comes back)
    pub fn activate(&mut self, key: u32) {
        let Some(i) = self.find(key) else { return };
        let mut w = self.wins.remove(i);
        w.min = false;
        self.wins.push(w);
        self.active = key;
    }

    // the topmost window that is not minimised becomes active (0: none)
    fn activate_top(&mut self) {
        self.active = self.wins.iter().rev().find(|w| !w.min).map_or(0, |w| w.key);
    }

    /// minimise the window (it keeps its place in the z-order; the topmost other one becomes
    /// active)
    pub fn minimize(&mut self, key: u32) {
        let Some(i) = self.find(key) else { return };
        self.wins[i].min = true;
        if self.active == key || self.active == 0 {
            self.activate_top();
        }
    }

    /// maximise the window (and activate it)
    pub fn maximize(&mut self, key: u32) {
        if let Some(i) = self.find(key) {
            self.wins[i].max = true;
            self.wins[i].snap = 0;
            self.activate(key);
        }
    }

    /// a minimised window comes back (as it was: maximised stays maximised); else a maximised
    /// or snapped one goes back to its normal place; it becomes active
    pub fn restore(&mut self, key: u32) {
        let Some(i) = self.find(key) else { return };
        let w = &mut self.wins[i];
        if w.min {
            w.min = false;
        } else {
            w.max = false;
            w.snap = 0;
        }
        self.activate(key);
    }

    /// maximise the window, or restore a maximised (or snapped) one
    pub fn toggle_max(&mut self, key: u32) {
        let Some(i) = self.find(key) else { return };
        let w = self.wins[i];
        if w.max || w.snap != 0 {
            self.wins[i].max = false;
            self.wins[i].snap = 0;
            self.activate(key);
        } else {
            self.maximize(key);
        }
    }

    /// snap the window to the start (DW_LEFT) or end (DW_RIGHT) half of the area, 0: back to
    /// its normal place
    pub fn snap(&mut self, key: u32, half: u32) {
        let Some(i) = self.find(key) else { return };
        let w = &mut self.wins[i];
        w.max = false;
        w.min = false;
        w.snap = match half {
            DW_LEFT => 1,
            DW_RIGHT => 2,
            _ => 0,
        };
        self.activate(key);
    }

    /// the next window (forward: the active one goes to the back and the one below it comes to
    /// the front; back: the one at the back comes to the front) among the windows that are not
    /// minimised -> the window now active
    pub fn cycle(&mut self, forward: bool) -> Option<u32> {
        self.cycle_in(forward, &|_| true)
    }

    // cycle among the windows `keep` accepts
    fn cycle_in(&mut self, forward: bool, keep: &dyn Fn(u32) -> bool) -> Option<u32> {
        let shown: Vec<usize> = (0..self.wins.len()).filter(|&i| !self.wins[i].min && keep(self.wins[i].key)).collect();
        if shown.is_empty() {
            return None;
        }
        if shown.len() > 1 {
            if forward {
                let top = *shown.last()?;
                let w = self.wins.remove(top);
                self.wins.insert(shown[0], w);
            } else {
                let w = self.wins.remove(shown[0]);
                self.wins.push(w);
            }
        }
        self.active = self.wins.iter().rev().find(|w| !w.min && keep(w.key)).map_or(0, |w| w.key);
        Some(self.active)
    }

    /// forget a window (a closed one: it comes back cascaded)
    pub fn forget(&mut self, key: u32) {
        if let Some(i) = self.find(key) {
            self.wins.remove(i);
            if self.active == key {
                self.activate_top();
            }
        }
    }

    /// the state as text (see DeskState)
    pub fn save(&self) -> String {
        use core::fmt::Write;
        let mut o = String::from("desk 1\n");
        for w in &self.wins {
            let r = w.rect;
            let _ = writeln!(o, "win {} {} {} {} {} {}", w.key, r[0], r[1], r[2], r[3], w.state());
        }
        let _ = writeln!(o, "active {}", self.active);
        o
    }

    /// a state saved with save -> None when the text is no desk state (version 1)
    pub fn load(text: &str) -> Option<DeskState> {
        let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#'));
        let mut w = lines.next()?.split_whitespace();
        if w.next()? != "desk" || w.next()? != "1" {
            return None;
        }
        let mut st = DeskState::default();
        let mut active = 0;
        for l in lines {
            let mut w = l.split_whitespace();
            let kind = w.next().unwrap_or("");
            let v: Vec<i64> = w.map(|x| x.parse::<i64>().unwrap_or(i64::MIN)).collect();
            if v.contains(&i64::MIN) {
                continue;
            }
            let int = |i: usize| v.get(i).copied().unwrap_or(0).clamp(-(SIZE_MAX as i64), SIZE_MAX as i64) as i32;
            match kind {
                "win" if v.len() >= 6 => {
                    let key = v[0].clamp(0, u32::MAX as i64) as u32;
                    if key == 0 || st.find(key).is_some() {
                        continue;
                    }
                    let s = v[5].clamp(0, 15) as u32;
                    let snap = if s & DW_LEFT != 0 { 1 } else if s & DW_RIGHT != 0 { 2 } else { 0 };
                    let rect = [int(1), int(2), int(3).max(1), int(4).max(1)];
                    st.wins.push(Win { key, rect, max: s & DW_MAX != 0, min: s & DW_MIN != 0, snap });
                }
                "active" if !v.is_empty() => active = v[0].clamp(0, u32::MAX as i64) as u32,
                _ => {}
            }
        }
        st.cascade = st.wins.len() as i32;
        if st.find(active).is_some_and(|i| !st.wins[i].min) {
            st.active = active;
        } else {
            st.activate_top();
        }
        Some(st)
    }
}

// the smallest outer size of a window (DIP; tbh: its title bar)
fn min_size(d: &DeskWin, tbh: i32) -> (i32, i32) {
    let w = if d.min[0] > 0 { d.min[0] } else { MIN_W };
    let h = if d.min[1] > 0 { d.min[1] + tbh } else { MIN_H.max(tbh * 2) };
    (w.clamp(1, SIZE_MAX), h.clamp(tbh + 1, SIZE_MAX))
}

// ------------------------------------------------------------------ the runtime part

// the frame's description of a window
struct Info {
    key: u32,
    title: String,
    icon: u32,
    flags: u32,
    /// the smallest outer size (DIP)
    min: [i32; 2],
}

// a window being moved (edges 0) or resized
#[derive(Clone, Copy)]
struct Drag {
    key: u32,
    edges: u32,
    /// the pointer at the start (px of the window, logical)
    ptr0: [i32; 2],
    /// the place at the start and now (DIP from the area's start corner)
    rect0: [i32; 4],
    cur: [i32; 4],
    /// where a release puts it: 0 there, DW_LEFT / DW_RIGHT a half, DW_MAX maximised
    zone: u32,
    /// it was maximised or snapped (the drag restored it)
    was: bool,
}

#[derive(Clone, Copy)]
enum Act {
    Activate(u32),
    Min(u32),
    ToggleMax(u32),
    Restore(u32),
    Close(u32),
    /// the window menu at its icon (true: the first row focused)
    Menu(u32),
    Cycle(bool),
}

/// one window area's state between the calls (and the passes)
#[derive(Default)]
pub(crate) struct Rt {
    id: u32,
    infos: Vec<Info>,
    /// the area and its part without the reserved strip (logical px)
    area: [i32; 4],
    work: [i32; 4],
    /// the windows of this pass, bottom to top, their outer rectangles (px) and the range of
    /// their hit boxes in the pass's list
    order: Vec<u32>,
    shown: Vec<(u32, [i32; 4], usize, usize)>,
    /// ... of the last pass (a press finds its window there)
    shown_prev: Vec<(u32, [i32; 4], usize, usize)>,
    next: usize,
    /// the window desk_next opened (0: none), whether its widgets are no Tab stops, the hit
    /// count before its content, its outer rectangle and whether it has grips
    open: u32,
    open_notab: bool,
    hits0: usize,
    open_rect: [i32; 4],
    open_place: [i32; 4],
    open_grips: bool,
    /// the index of its record in Ui::sems (its content is counted into Sem::max at its end)
    open_rec: usize,
    drag: Option<Drag>,
    acts: Vec<Act>,
    evs: Vec<DeskEv>,
    /// the widget focused last in each window
    wfocus: Vec<(u32, u32)>,
    /// the window whose content gets the focus when it is emitted next, the focus visual then
    focus_into: u32,
    focus_kb: bool,
    /// the focus was inside the area in this pass / the last one; the focus at the last
    /// desk_end
    focus_here: bool,
    focus_in: bool,
    last_focus: u32,
    /// the active window at the last desk_end (Activate events, the focus follows)
    last_active: u32,
    /// the window menu opened by a right click (anchored to the pointer)
    menu_point: u32,
    /// the taskbar's menu opened by a right click
    tmenu_point: u32,
}

impl Rt {
    fn info(&self, key: u32) -> Option<&Info> {
        self.infos.iter().find(|i| i.key == key)
    }
    fn flags(&self, key: u32) -> u32 {
        self.info(key).map_or(0, |i| i.flags)
    }
    fn present(&self, key: u32) -> bool {
        self.infos.iter().any(|i| i.key == key)
    }
}

/// the window areas of the app (Mods)
#[derive(Default)]
pub(crate) struct S {
    desks: Vec<Rt>,
    /// the areas between desk_begin and desk_end
    cur: Vec<u32>,
}

// a library text (German source; i18n::tr)
fn tr(s: &'static str) -> &'static str {
    super::i18n::tr(s)
}

// the name of a window for screen readers and the taskbar: its title, its state after it
fn named(title: &str, state: u32) -> String {
    if state & DW_MIN != 0 {
        alloc::format!("{} ({})", title, tr("minimiert"))
    } else if state & DW_MAX != 0 {
        alloc::format!("{} ({})", title, tr("maximiert"))
    } else {
        String::from(title)
    }
}

impl Ui {
    fn dw_take(&mut self, id: u32) -> Rt {
        let d = &mut self.m.desk.desks;
        match d.iter().position(|r| r.id == id) {
            Some(i) => d.swap_remove(i),
            None => Rt { id, ..Rt::default() },
        }
    }

    fn dw_put(&mut self, rt: Rt) {
        self.m.desk.desks.push(rt);
    }

    // the title bar height (DIP): the style's dialog window title bar
    fn dw_tbh(&self) -> i32 {
        let v = self.style.metric(SM_WIN_TBH_DIALOG);
        if v > 0 { v } else { 32 }
    }

    // vk with exactly these modifiers pressed this pass (consumed; a text box keeps its keys)
    fn dw_key(&mut self, vk: u32, mods: u32) -> bool {
        let m3 = MOD_SHIFT | MOD_CTRL | MOD_ALT;
        let k = self.wp.ctx.keyq.iter().position(|&k| (k as i32) >= 0 && k & 0xFFFF == vk && (k >> 16) & m3 == mods);
        match self.text_key_filter(k) {
            Some(i) => {
                self.wp.ctx.keyq[i] |= 0x8000_0000;
                true
            }
            None => false,
        }
    }

    // px -> DIP, rounded the same way on both sides of 0 (deltas, places left of the area)
    fn dw_dip(&self, px: i32) -> i32 {
        super::layout::px2dip_at(px, self.wp.rt.dpi)
    }

    // DIP from the area's start corner -> logical px of the window
    fn dw_px(&self, rt: &Rt, r: [i32; 4]) -> [i32; 4] {
        [rt.area[0] + self.dp(r[0]), rt.area[1] + self.dp(r[1]), self.dp(r[2]), self.dp(r[3])]
    }

    // the work area (the area without the reserved strip) in DIP from the area's start corner
    fn dw_work(&self, rt: &Rt) -> [i32; 4] {
        let w = rt.work;
        [self.dw_dip(w[0] - rt.area[0]), self.dw_dip(w[1] - rt.area[1]), self.dw_dip(w[2]), self.dw_dip(w[3])]
    }

    // a normal place kept reachable inside the work area (DIP)
    fn dw_reach(&self, rt: &Rt, r: [i32; 4]) -> [i32; 4] {
        let wk = self.dw_work(rt);
        let tbh = self.dw_tbh();
        let keep = KEEP.min(r[2]);
        let x = r[0].min(wk[0] + wk[2] - keep).max(wk[0] + keep - r[2]);
        let y = r[1].min(wk[1] + wk[3] - tbh).max(wk[1]);
        [x, y, r[2], r[3]]
    }

    // where a window shows (DIP from the area's start corner) and whether it fills the work
    // area (maximised: no frame); a drag shows its own place
    fn dw_place(&self, rt: &Rt, w: &Win) -> ([i32; 4], bool) {
        if let Some(d) = rt.drag.filter(|d| d.key == w.key) {
            return (d.cur, false);
        }
        let wk = self.dw_work(rt);
        if w.max {
            return (wk, true);
        }
        match w.snap {
            1 => ([wk[0], wk[1], wk[2] / 2, wk[3]], false),
            2 => ([wk[0] + wk[2] / 2, wk[1], wk[2] - wk[2] / 2, wk[3]], false),
            _ => (self.dw_reach(rt, w.rect), false),
        }
    }

    // ------------------------------------------------------------------ app API

    /// desk_begin(id, &mut st, wins): open the window area (always pair with desk_end; fill it
    /// with desk_next).  It takes the next place of the box (the whole rest when no size was
    /// set); wins: every open window, each frame (titles and icons may change), in the order
    /// the taskbar shows them.
    pub fn desk_begin(&mut self, id: u32, st: &mut DeskState, wins: &[DeskWin]) {
        let mut rt = self.dw_take(id);
        let cur = &mut self.m.desk.cur;
        cur.retain(|&c| c != id); // (a desk_end the app left out)
        cur.push(id);
        let tbh = self.dw_tbh();
        rt.infos = wins
            .iter()
            .filter(|d| d.key != 0)
            .map(|d| {
                let (mw, mh) = min_size(d, tbh);
                Info { key: d.key, title: String::from(d.title), icon: d.icon, flags: d.flags, min: [mw, mh] }
            })
            .collect();
        let l = &self.wp.lay;
        if l.next_w == 0 && l.next_h == 0 && l.next_rect_set == 0 {
            self.next_size(SIZE_FILL, SIZE_FILL);
        }
        self.box_begin(pid(id, P_AREA, 0), BOX_V | BOX_FIXED | BOX_CLIP, 0);
        rt.area = self.box_rect();
        let a = rt.area;
        let rv = st.reserve.map(|v| self.dp(v));
        rt.work = [a[0] + rv[0], a[1] + rv[1], (a[2] - rv[0] - rv[2]).max(1), (a[3] - rv[1] - rv[3]).max(1)];
        // new windows cascade inside the work area (0: not measured yet in a first pass)
        let wk = rt.work;
        let dims = if a[2] > 0 && a[3] > 0 { [self.dw_dip(wk[2]), self.dw_dip(wk[3])] } else { [0, 0] };
        st.adopt(wins, dims, tbh);
        // the background
        let sh = self.style.shell();
        let mut sp = Sp::default();
        sp.place(a);
        sh.desk_bg(self, &sp);
        // a drag follows the pointer, ends with the release; Esc cancels it
        self.dw_drag_update(&mut rt, st);
        // a press on a window brings it to the front
        self.dw_press(&mut rt, st);
        self.dw_sync_active(&mut rt, st, false);
        rt.shown_prev = core::mem::take(&mut rt.shown);
        rt.order = st.wins.iter().filter(|w| !w.min && rt.present(w.key)).map(|w| w.key).collect();
        // (the focus memory of windows that went away)
        let infos = &rt.infos;
        rt.wfocus.retain(|e| infos.iter().any(|i| i.key == e.0));
        rt.next = 0;
        rt.open = 0;
        rt.focus_here = false;
        self.dw_put(rt);
    }

    /// desk_next(&mut st) -> the next window to fill (None: done), from the back to the
    /// front; its content goes into a vertical box (it clips) until the next call
    pub fn desk_next(&mut self, st: &mut DeskState) -> Option<u32> {
        let id = *self.m.desk.cur.last()?;
        let mut rt = self.dw_take(id);
        self.dw_close(&mut rt, st);
        let mut out = None;
        if rt.next < rt.order.len() {
            let key = rt.order[rt.next];
            rt.next += 1;
            self.dw_window(&mut rt, st, key);
            out = Some(key);
        }
        self.dw_put(rt);
        out
    }

    /// desk_end(&mut st) -> what the user did in this pass (and the taskbar since the last
    /// desk_end): close requests, activation, moves, resizes, state changes
    pub fn desk_end(&mut self, st: &mut DeskState) -> Vec<DeskEv> {
        let Some(id) = self.m.desk.cur.pop() else { return Vec::new() };
        let mut rt = self.dw_take(id);
        self.dw_close(&mut rt, st);
        // the keys, while the focus is inside the area (or nowhere)
        let focus = self.wp.ctx.focus;
        if rt.drag.is_none() && (rt.focus_here || rt.focus_in || focus == 0) {
            let a = st.active;
            if self.dw_key(VK_F6, MOD_ALT) {
                rt.acts.push(Act::Cycle(true));
            }
            if self.dw_key(VK_F6, MOD_ALT | MOD_SHIFT) {
                rt.acts.push(Act::Cycle(false));
            }
            if a != 0 && rt.present(a) {
                let f = rt.flags(a);
                if f & (DWF_NOMAX | DWF_FIXED) == 0 && self.dw_key(VK_F10, MOD_ALT) {
                    rt.acts.push(Act::ToggleMax(a));
                }
                if f & DWF_NOMIN == 0 && self.dw_key(VK_F9, MOD_ALT) {
                    rt.acts.push(Act::Min(a));
                }
                if self.dw_key(VK_OEM_MINUS, MOD_ALT) || self.dw_key(VK_SUBTRACT, MOD_ALT) {
                    rt.acts.push(Act::Menu(a));
                }
            }
        }
        let acts = core::mem::take(&mut rt.acts);
        let mut by_focus = false;
        for a in acts {
            by_focus |= self.dw_apply(&mut rt, st, a);
        }
        self.dw_sync_active(&mut rt, st, by_focus);
        self.box_end(); // the area
        rt.focus_in = rt.focus_here;
        rt.last_focus = self.wp.ctx.focus;
        let evs = core::mem::take(&mut rt.evs);
        self.dw_put(rt);
        evs
    }

    // ------------------------------------------------------------------ input

    // the active window changed (by anyone): Activate, and its content gets the focus (unless
    // the focus arriving there activated it)
    fn dw_sync_active(&mut self, rt: &mut Rt, st: &mut DeskState, by_focus: bool) {
        // a window the app does not describe (closed, the state keeps its place) is no
        // active one
        if st.active != 0 && (!rt.present(st.active) || st.state(st.active) & DW_MIN != 0) {
            st.active = st.wins.iter().rev().find(|w| !w.min && rt.present(w.key)).map_or(0, |w| w.key);
        }
        if st.active == rt.last_active {
            return;
        }
        rt.last_active = st.active;
        if st.active == 0 {
            return;
        }
        rt.evs.push(DeskEv::Activate(st.active));
        if !by_focus {
            rt.focus_into = st.active;
        }
        self.repass = true;
    }

    // a press of any button on a window (the window whose hit box took it in the last pass:
    // also its grips outside the frame) brings it to the front
    fn dw_press(&mut self, rt: &mut Rt, st: &mut DeskState) {
        let c = &self.wp.ctx;
        if c.pressed & (1 << BTN_LEFT | 1 << BTN_RIGHT | 1 << BTN_MIDDLE) == 0 || c.modal_prev > L_BASE {
            return;
        }
        let p = if c.pressed & (1 << BTN_LEFT) != 0 { [c.press_x, c.press_y] } else { [c.mx, c.my] };
        if !inside(rt.area, p) {
            return;
        }
        // a popup (a menu, or the layer that closes them) over the point takes the press
        let hits = &c.hits[c.hit_cur ^ 1];
        let at = |h: &super::ctx::Hit| p[0] >= h.x0 && p[0] < h.x1 && p[1] >= h.y0 && p[1] < h.y1;
        if hits.iter().any(|h| h.layer as u32 > L_BASE && at(h)) {
            return;
        }
        let Some(top) = hits.iter().rposition(at) else { return };
        let Some(&(key, ..)) = rt.shown_prev.iter().find(|e| top >= e.2 && top < e.3) else { return };
        if st.active != key || st.wins.last().is_some_and(|w| w.key != key) {
            st.activate(key);
            self.repass = true;
        }
    }

    // follow the pointer with the window being dragged; the release commits it
    fn dw_drag_update(&mut self, rt: &mut Rt, st: &mut DeskState) {
        let Some(mut d) = rt.drag else { return };
        if !st.contains(d.key) || !rt.present(d.key) {
            rt.drag = None;
            return;
        }
        // (also while a text box has the focus: the drag holds the pointer)
        if self.kb_chord(VK_ESCAPE, 0) {
            rt.drag = None;
            self.wp.ctx.active = 0; // the release is no click
            self.repass = true;
            return;
        }
        let (mx, my) = self.mouse();
        let held = self.wp.ctx.mbtn & (1 << BTN_LEFT) != 0;
        let (dx, dy) = (self.dw_dip(mx - d.ptr0[0]), self.dw_dip(my - d.ptr0[1]));
        let wk = self.dw_work(rt);
        let r0 = d.rect0;
        let min = rt.info(d.key).map_or([MIN_W, MIN_H], |i| i.min);
        let mut r = r0;
        d.zone = 0;
        if d.edges == 0 {
            r[0] = r0[0] + dx;
            r[1] = r0[1] + dy;
            // the edges stick to the work area's
            let snap = |v: i32, len: i32, lo: i32, hi: i32| {
                if (v - lo).abs() <= MAGNET {
                    lo
                } else if (v + len - hi).abs() <= MAGNET {
                    hi - len
                } else {
                    v
                }
            };
            r[0] = snap(r[0], r[2], wk[0], wk[0] + wk[2]);
            r[1] = snap(r[1], r[3], wk[1], wk[1] + wk[3]);
            r = self.dw_reach(rt, r);
            // the pointer at the area's edges: a half or the whole area
            let f = rt.flags(d.key);
            let a = rt.area;
            let z = self.dp(ZONE);
            if f & DWF_FIXED == 0 {
                if mx <= a[0] + z {
                    d.zone = DW_LEFT;
                } else if mx >= a[0] + a[2] - 1 - z {
                    d.zone = DW_RIGHT;
                }
            }
            if d.zone == 0 && my <= a[1] + z && f & (DWF_NOMAX | DWF_FIXED) == 0 {
                d.zone = DW_MAX;
            }
        } else {
            // the dragged edges, inside the work area, not below the minimum size
            let (x1, y1) = (r0[0] + r0[2], r0[1] + r0[3]);
            if d.edges & E_LEFT != 0 {
                let x = (r0[0] + dx).max(wk[0].min(r0[0])).min(x1 - min[0]);
                r[0] = x;
                r[2] = x1 - x;
            }
            if d.edges & E_RIGHT != 0 {
                r[2] = (r0[2] + dx).min((wk[0] + wk[2]).max(x1) - r0[0]).max(min[0]);
            }
            if d.edges & E_TOP != 0 {
                let y = (r0[1] + dy).max(wk[1].min(r0[1])).min(y1 - min[1]);
                r[1] = y;
                r[3] = y1 - y;
            }
            if d.edges & E_BOTTOM != 0 {
                r[3] = (r0[3] + dy).min((wk[1] + wk[3]).max(y1) - r0[1]).max(min[1]);
            }
        }
        if held {
            if r != d.cur || rt.drag.is_some_and(|o| o.zone != d.zone) {
                self.repass = true;
            }
            d.cur = r;
            rt.drag = Some(d);
            return;
        }
        // released: the new place
        rt.drag = None;
        self.repass = true;
        let Some(i) = st.find(d.key) else { return };
        let w = &mut st.wins[i];
        if d.zone != 0 && d.edges == 0 {
            // (the normal place stays what it was)
            w.max = d.zone == DW_MAX;
            w.snap = match d.zone {
                DW_LEFT => 1,
                DW_RIGHT => 2,
                _ => 0,
            };
            rt.evs.push(DeskEv::State(d.key));
            return;
        }
        w.rect = r;
        w.max = false;
        w.snap = 0;
        if d.was {
            rt.evs.push(DeskEv::State(d.key));
        }
        if r != r0 || d.was {
            rt.evs.push(if d.edges == 0 { DeskEv::Move(d.key) } else { DeskEv::Resize(d.key) });
        }
    }

    // start moving (edges 0) or resizing window key from its place r (DIP), the pointer at the
    // press
    fn dw_drag_start(&mut self, rt: &mut Rt, st: &DeskState, key: u32, edges: u32, r: [i32; 4]) {
        let Some(i) = st.find(key) else { return };
        let w = st.wins[i];
        let (px, py) = (self.wp.ctx.press_x, self.wp.ctx.press_y);
        let mut rect0 = r;
        let was = edges == 0 && (w.max || w.snap != 0);
        if was {
            // back to the normal size under the pointer, at the same share of the title bar
            let n = w.rect;
            let (ax, ay) = (self.dw_dip(px - rt.area[0]), self.dw_dip(py - rt.area[1]));
            let share = if r[2] > 0 { ((ax - r[0]) as i64 * 1024 / r[2] as i64) as i32 } else { 512 };
            let x = ax - (n[2] as i64 * share as i64 / 1024) as i32;
            let y = ay - (ay - r[1]).min(self.dw_tbh() / 2);
            rect0 = self.dw_reach(rt, [x, y, n[2], n[3]]);
        }
        rt.drag = Some(Drag { key, edges, ptr0: [px, py], rect0, cur: rect0, zone: 0, was });
        self.wp.ctx.last_click_id = 0; // a drag is no first click of a double click
        self.repass = true;
    }

    // ------------------------------------------------------------------ a window

    // the frame, the title bar, the buttons, the menu and the content box of window key
    fn dw_window(&mut self, rt: &mut Rt, st: &mut DeskState, key: u32) {
        let id = rt.id;
        let Some(i) = st.find(key) else { return };
        let w = st.wins[i];
        let (title, icon, flags) = rt.info(key).map_or((String::new(), 0, 0), |f| (f.title.clone(), f.icon, f.flags));
        let active = st.active == key;
        let (place, max) = self.dw_place(rt, &w);
        let r = self.dw_px(rt, place);
        let sh = self.style.shell();
        let stl = self.style;
        // where a release of the drag puts it
        if let Some(d) = rt.drag.filter(|d| d.key == key && d.zone != 0) {
            let wk = self.dw_work(rt);
            let p = match d.zone {
                DW_LEFT => [wk[0], wk[1], wk[2] / 2, wk[3]],
                DW_RIGHT => [wk[0] + wk[2] / 2, wk[1], wk[2] - wk[2] / 2, wk[3]],
                _ => wk,
            };
            let mut sp = Sp { value: (d.zone == DW_MAX) as i32, ..Sp::default() };
            sp.place(self.dw_px(rt, p));
            sh.desk_snap(self, &sp);
        }
        // the record: everything of the window lies in its box, one level deeper (a11y.rs)
        let state = if max { DW_MAX } else { w.state() & !DW_MIN };
        let label = named(&title, state);
        self.wp.ctx.last_rect = r;
        self.wp.ctx.last_state = if active { SS_SELECTED as u32 } else { 0 };
        rt.open_rec = self.sems.len();
        let rec = self.sem(K_DIALOG, pid(id, P_WIN, key), Some(&label), state as i32);
        rec.min = super::a11y::SEM_SCOPE;
        self.next_rect_px(r[0], r[1], r[2], r[3]);
        self.box_begin(pid(id, P_BOX, key), BOX_V | BOX_FIXED, 0);
        // the window takes the pointer: nothing below it is hit
        self.wp.ctx.last_rect = r;
        let h0 = self.wp.ctx.hits[self.wp.ctx.hit_cur].len();
        self.hit(pid(id, P_WIN, key), HB_BLOCK);
        rt.shown.push((key, r, h0, usize::MAX));
        let mut sp = Sp { value: active as i32, flags: max as u32, ..Sp::default() };
        sp.place(r);
        sh.desk_window(self, &sp);
        // the title bar: icon and title, the buttons at the end (the style's SM_CAP_LEFT: at the
        // start, close first)
        let b = if max { 0 } else { self.hair() };
        let tbh = self.dp(self.dw_tbh()).min((r[3] - b * 2).max(0));
        let bar = [r[0] + b, r[1] + b, (r[2] - b * 2).max(0), tbh];
        let capw = {
            let v = stl.metric(SM_CAP_W_TOOL);
            self.dp(if v > 0 { v } else { 40 })
        };
        let left = stl.metric(SM_CAP_LEFT) != 0;
        let mut btns: Vec<u32> = Vec::new();
        if flags & DWF_NOMIN == 0 {
            btns.push(P_MIN);
        }
        if flags & (DWF_NOMAX | DWF_FIXED) == 0 {
            btns.push(P_MAX);
        }
        if flags & DWF_NOCLOSE == 0 {
            if left {
                btns.insert(0, P_CLOSE);
            } else {
                btns.push(P_CLOSE);
            }
        }
        let nb = (btns.len() as i32).min(((bar[2] - self.dp(24)) / capw.max(1)).max(0));
        let bw = capw * nb;
        let bx = if left { bar[0] } else { bar[0] + bar[2] - bw };
        let pad = {
            let v = stl.metric(SM_WC_TITLE_PAD);
            self.dp(if v > 0 { v } else { 12 })
        };
        let tx = if left { bar[0] + bw + pad } else { bar[0] + pad };
        let tw = (bar[2] - bw - pad * 2).max(0);
        let mut tsp = Sp { label: Some(&title), icon, value: active as i32, ..Sp::default() };
        tsp.place(bar);
        tsp.s[0] = tx;
        tsp.s[1] = tw;
        sh.desk_titlebar(self, &tsp);
        let trect = if left { [bar[0] + bw, bar[1], bar[2] - bw, tbh] } else { [bar[0], bar[1], bar[2] - bw, tbh] };
        let tid = pid(id, P_TITLE, key);
        let ev = self.hit_rect(tid, trect[0], trect[1], trect[2], trect[3], 0);
        if ev & EV_HELD != 0 && ev & EV_DRAG != 0 && rt.drag.is_none() {
            self.dw_drag_start(rt, st, key, 0, place);
        } else if ev & EV_DOUBLE != 0 && flags & (DWF_NOMAX | DWF_FIXED) == 0 {
            rt.acts.push(Act::ToggleMax(key));
        }
        let menu = pid(id, P_MENU, key);
        if ev & EV_RCLICK != 0 {
            self.popup_open(menu);
            rt.menu_point = menu;
        }
        self.wp.ctx.last_rect = trect;
        self.wp.ctx.last_state = 0;
        self.sem(K_TITLEBAR, tid, Some(&title), 0);
        // the icon opens the window menu (it hangs below the icon)
        let isz = self.dp(16);
        let irect = [tx - pad / 2, bar[1], isz + pad, tbh];
        if icon != 0 && tw > isz {
            let iev = self.hit_rect(pid(id, P_ICON, key), irect[0], irect[1], irect[2], irect[3], 0);
            if iev & EV_ACT != 0 {
                self.popup_open(menu);
                rt.menu_point = 0;
            }
        } else {
            self.wp.ctx.last_rect = irect;
        }
        self.dw_menu(rt, st, key, flags, menu);
        // the buttons
        for (k, &part) in btns.iter().take(nb as usize).enumerate() {
            let x = bx + capw * k as i32;
            let bid = pid(id, part, key);
            let (hb, ic, lab) = match part {
                P_MIN => (HB_MIN, IC_CHROME_MIN, tr("Minimieren")),
                P_MAX if state & DW_MAX != 0 || (w.snap != 0 && !max) => (HB_MAX, IC_CHROME_RESTORE, tr("Wiederherstellen")),
                P_MAX => (HB_MAX, IC_CHROME_MAX, tr("Maximieren")),
                _ => (HB_CLOSE, IC_CHROME_CLOSE, tr("Schließen")),
            };
            let bev = self.hit_rect(bid, x, bar[1], capw, tbh, 0);
            let mut bsp = Sp { ev: bev, flags: hb, icon: ic, value: active as i32, ..Sp::default() };
            bsp.place([x, bar[1], capw, tbh]);
            sh.desk_button(self, &bsp);
            // named with the window's title (the buttons of every window can be told apart)
            self.sem(K_BUTTON, bid, Some(&alloc::format!("{}: {}", lab, title)), 0);
            if bev & EV_ACT != 0 {
                rt.acts.push(match part {
                    P_MIN => Act::Min(key),
                    P_MAX => Act::ToggleMax(key),
                    _ => Act::Close(key),
                });
            }
        }
        // the content
        let c = [r[0] + b, r[1] + b + tbh, (r[2] - b * 2).max(0), (r[3] - b * 2 - tbh).max(0)];
        self.next_rect_px(c[0], c[1], c[2], c[3]);
        self.box_begin(pid(id, P_CONTENT, key), BOX_V | BOX_CLIP | BOX_FIXED, 0);
        self.focus_pane_begin(pid(id, P_PANE, key));
        rt.open_notab = !active;
        if !active {
            self.kb_notab(true); // only the active window has Tab stops
        }
        rt.open = key;
        rt.hits0 = self.wp.ctx.hits[self.wp.ctx.hit_cur].len();
        rt.open_rect = r;
        rt.open_place = place;
        rt.open_grips = !max && flags & DWF_FIXED == 0;
    }

    // the window menu of window key (popup `menu`): Wiederherstellen, Minimieren, Maximieren,
    // Schließen
    fn dw_menu(&mut self, rt: &mut Rt, st: &DeskState, key: u32, flags: u32, menu: u32) {
        let fl = if rt.menu_point == menu { FL_POINT } else { FL_BELOW };
        if self.flyout_begin(menu, fl) {
            let s = st.state(key);
            let id = rt.id;
            let rows: [(&'static str, bool, Act); 3] = [
                (tr("Wiederherstellen"), s & (DW_MAX | DW_MIN | DW_LEFT | DW_RIGHT) != 0, Act::Restore(key)),
                (tr("Minimieren"), flags & DWF_NOMIN == 0 && s & DW_MIN == 0, Act::Min(key)),
                (tr("Maximieren"), flags & (DWF_NOMAX | DWF_FIXED) == 0 && s & DW_MAX == 0, Act::ToggleMax(key)),
            ];
            for (k, (label, on, act)) in rows.into_iter().enumerate() {
                if !on {
                    self.next_flags(ST_DISABLED);
                }
                if self.menu_item(pid(id, P_MITEM + k as u32, key), label) & EV_ACT != 0 && on {
                    rt.acts.push(act);
                }
            }
            self.menu_sep();
            if flags & DWF_NOCLOSE != 0 {
                self.next_flags(ST_DISABLED);
            }
            if self.menu_item(pid(id, P_MITEM + 3, key), tr("Schließen")) & EV_ACT != 0 && flags & DWF_NOCLOSE == 0 {
                rt.acts.push(Act::Close(key));
            }
        } else if rt.menu_point == menu {
            rt.menu_point = 0;
        }
        self.flyout_end();
    }

    // close what desk_next opened: the focus, the resize grips (they win over the content's
    // edge), the window's box
    fn dw_close(&mut self, rt: &mut Rt, st: &mut DeskState) {
        let key = core::mem::take(&mut rt.open);
        if key == 0 {
            return;
        }
        let id = rt.id;
        if rt.open_notab {
            self.kb_notab(false);
        }
        self.focus_pane_end();
        self.box_end(); // the content
        // the focus inside the window: remembered; arriving in a window behind, it brings it
        // to the front
        let focus = self.wp.ctx.focus;
        let (here, first, remembered) = {
            let c = &self.wp.ctx;
            let hits = &c.hits[c.hit_cur];
            let from = rt.hits0.min(hits.len());
            let mine = &hits[from..];
            let here = focus != 0 && mine.iter().any(|h| h.id == focus);
            let ok = |h: &&super::ctx::Hit| h.flags as u32 & (HB_FOCUS | HB_DISABLED) == HB_FOCUS && !h.continuation;
            let first = mine.iter().find(|h| ok(h) && h.flags as u32 & HB_NOTAB == 0).or_else(|| mine.iter().find(ok)).map(|h| h.id);
            let want = rt.wfocus.iter().find(|e| e.0 == key).map_or(0, |e| e.1);
            let remembered = if want != 0 && mine.iter().any(|h| h.id == want && ok(&h)) { Some(want) } else { None };
            (here, first, remembered)
        };
        if here {
            rt.focus_here = true;
            match rt.wfocus.iter_mut().find(|e| e.0 == key) {
                Some(e) => e.1 = focus,
                None => rt.wfocus.push((key, focus)),
            }
            if rt.focus_into == key {
                rt.focus_into = 0;
            }
            if st.active != key && focus != rt.last_focus && rt.drag.is_none() {
                rt.acts.push(Act::Activate(key));
            }
        } else if rt.focus_into == key {
            rt.focus_into = 0;
            match remembered.or(first) {
                Some(f) => {
                    let kb = rt.focus_kb;
                    self.focus(f);
                    self.wp.ctx.focus_visible = kb;
                    rt.focus_here = true;
                }
                // nothing to focus: the keys go to no window behind it
                None if rt.focus_in => self.wp.ctx.focus = 0,
                None => {}
            }
            rt.focus_kb = false;
        }
        // the resize grips: bands along the edges (mostly outside the frame), squares at the
        // corners
        if rt.open_grips {
            let r = rt.open_rect;
            let (o, n) = (self.dp(GRIP_OUT), self.dp(GRIP_IN));
            let cs = {
                let v = self.style.metric(SM_GRIP_CORNER);
                self.dp(if v > 0 { v } else { 12 })
            };
            let (x0, y0, x1, y1) = (r[0] - o, r[1] - o, r[0] + r[2] + o, r[1] + r[3] + o);
            let place = rt.open_place;
            for (k, &(edges, cur)) in GRIPS.iter().enumerate() {
                let g = match edges {
                    E_TOP => [x0 + cs, y0, x1 - x0 - cs * 2, o],
                    E_BOTTOM => [x0 + cs, y1 - o - n, x1 - x0 - cs * 2, o + n],
                    E_LEFT => [x0, y0 + cs, o + n, y1 - y0 - cs * 2],
                    E_RIGHT => [x1 - o - n, y0 + cs, o + n, y1 - y0 - cs * 2],
                    _ => {
                        let gx = if edges & E_LEFT != 0 { x0 } else { x1 - cs };
                        let gy = if edges & E_TOP != 0 { y0 } else { y1 - cs };
                        [gx, gy, cs, cs]
                    }
                };
                // right to left: the logical corners show mirrored, their arrows too
                let cur = match cur {
                    CUR_SIZE_NWSE if self.rtl => CUR_SIZE_NESW,
                    CUR_SIZE_NESW if self.rtl => CUR_SIZE_NWSE,
                    c => c,
                };
                let gid = pid(id, P_GRIP + k as u32, key);
                let ev = self.hit_rect(gid, g[0], g[1], g[2], g[3], cur << HB_CURSOR_SHIFT);
                if ev & EV_HELD != 0 && rt.drag.is_none() {
                    self.dw_drag_start(rt, st, key, edges, place);
                }
            }
        }
        self.box_end(); // the window
        let h1 = self.wp.ctx.hits[self.wp.ctx.hit_cur].len();
        if let Some(e) = rt.shown.iter_mut().rev().find(|e| e.0 == key) {
            e.3 = h1;
        }
        // the window's record holds the records emitted since (of its OS window): a11y.rs
        let at = rt.open_rec;
        if at < self.sems.len() && self.sems[at].id == pid(id, P_WIN, key) {
            let win = self.sems[at].win;
            let n = self.sems[at + 1..].iter().filter(|s| s.win == win).count();
            self.sems[at].max = n as i32;
        }
    }

    // ------------------------------------------------------------------ changes

    // apply one change -> true when the focus arriving in a window activated it
    fn dw_apply(&mut self, rt: &mut Rt, st: &mut DeskState, a: Act) -> bool {
        self.repass = true;
        match a {
            Act::Activate(k) => {
                st.activate(k);
                return true;
            }
            Act::Min(k) => {
                if st.state(k) & DW_MIN == 0 {
                    st.minimize(k);
                    rt.evs.push(DeskEv::State(k));
                }
            }
            Act::ToggleMax(k) => {
                st.toggle_max(k);
                rt.evs.push(DeskEv::State(k));
            }
            Act::Restore(k) => {
                st.restore(k);
                rt.evs.push(DeskEv::State(k));
            }
            Act::Close(k) => rt.evs.push(DeskEv::Close(k)),
            Act::Menu(k) => {
                let menu = pid(rt.id, P_MENU, k);
                self.popup_open(menu);
                self.pp_flag(menu, super::popup::PPF_FOCUS);
                self.wp.ctx.focus_visible = true;
                rt.menu_point = 0;
            }
            Act::Cycle(fwd) => {
                // only the windows the app describes take part
                let before = st.active;
                let infos = &rt.infos;
                if st.cycle_in(fwd, &|k| infos.iter().any(|i| i.key == k)).is_some() && st.active != before {
                    rt.focus_kb = true;
                }
            }
        }
        false
    }

    // ------------------------------------------------------------------ taskbar

    /// desk_taskbar_begin(id): open a taskbar for desk `id` (always pair with
    /// desk_taskbar_end): a row with the style's taskbar surface; the app's items and
    /// desk_taskbar go inside.  Inside desk_begin .. desk_end without next_size / next_rect it
    /// takes the area's reserved strip (DeskState::set_reserve: the bottom one, else the top
    /// one), else the next place, TASK_H DIP high unless next_size says otherwise.
    pub fn desk_taskbar_begin(&mut self, id: u32) {
        let l = &self.wp.lay;
        let no_size = l.next_w == 0 && l.next_h == 0 && l.next_rect_set == 0;
        let open = self.m.desk.cur.contains(&id);
        let mut placed = false;
        if no_size && open {
            if let Some(rt) = self.m.desk.desks.iter().find(|r| r.id == id) {
                let (a, w) = (rt.area, rt.work);
                let bottom = a[1] + a[3] - (w[1] + w[3]);
                let top = w[1] - a[1];
                if bottom > 0 {
                    self.next_rect_px(a[0], w[1] + w[3], a[2], bottom);
                    placed = true;
                } else if top > 0 {
                    self.next_rect_px(a[0], a[1], a[2], top);
                    placed = true;
                }
            }
        }
        if no_size && !placed {
            self.next_size(0, TASK_H);
        }
        self.box_begin(pid(id, P_BAR, 0), BOX_H | BOX_XCENTER | BOX_BLOCK | pad(4), 4);
        let mut sp = Sp::default();
        sp.place(self.wp.ctx.last_rect);
        self.style.shell().taskbar(self, &sp);
        self.wp.ctx.last_state = 0;
        self.sem(K_BOX, pid(id, P_BAR, 0), Some(tr("Taskleiste")), 0);
        self.focus_pane_begin(pid(id, P_BAR, 0)); // F6 reaches it
    }

    /// desk_taskbar_end
    pub fn desk_taskbar_end(&mut self) {
        self.focus_pane_end();
        self.box_end();
    }

    /// desk_taskbar(id, &mut st, flags): the buttons of desk `id`'s windows (DTB_*), in the
    /// order of desk_begin's list; without a size they take the room the row leaves.  Their
    /// changes apply at once; desk_end reports them.
    pub fn desk_taskbar(&mut self, id: u32, st: &mut DeskState, flags: u32) {
        let l = &self.wp.lay;
        if l.next_w == 0 && l.next_h == 0 && l.next_rect_set == 0 {
            // the rest of the row, as high as the row
            let h = self.dw_dip(self.box_rect()[3]).max(1);
            self.next_size(SIZE_FILL, h);
        }
        self.box_begin(pid(id, P_TASK, 0), BOX_H | BOX_FIXED | BOX_CLIP, 0);
        let area = self.box_rect();
        let mut rt = self.dw_take(id);
        let sh = self.style.shell();
        let wins: Vec<(u32, String, u32, u32)> = rt
            .infos
            .iter()
            .filter(|i| i.flags & DWF_NOTASK == 0 && st.contains(i.key))
            .map(|i| (i.key, i.title.clone(), i.icon, i.flags))
            .collect();
        // the widths: natural, between TASK_MIN and TASK_MAX; what does not fit goes into the
        // "Weitere Fenster" menu (the active window stays in the row)
        let icons = flags & DTB_ICONS != 0;
        let gap = self.dp(TASK_GAP);
        let mut ws: Vec<i32> = Vec::with_capacity(wins.len());
        for (_, t, ic, _) in &wins {
            let mut tsp = Sp { label: Some(t), icon: *ic, flags, ..Sp::default() };
            let (w, _) = sh.taskbar_button_size(self, &mut tsp);
            ws.push(if icons { w } else { w.clamp(self.dp(TASK_MIN), self.dp(TASK_MAX)) });
        }
        let total: i32 = ws.iter().sum::<i32>() + gap * (ws.len() as i32 - 1).max(0);
        let mut shown: Vec<usize> = (0..wins.len()).collect();
        let mut over: Vec<usize> = Vec::new();
        if total > area[2] && !wins.is_empty() {
            // shrink to the minimum first
            let minw = if icons { ws.iter().copied().max().unwrap_or(0) } else { self.dp(TASK_MIN) };
            let n = wins.len() as i32;
            if minw * n + gap * (n - 1) <= area[2] {
                let each = ((area[2] - gap * (n - 1)) / n).max(minw);
                for w in ws.iter_mut() {
                    *w = (*w).min(each);
                }
            } else {
                let ow = self.dp(OVER_W);
                let k = ((area[2] - ow + gap) / (minw + gap)).max(0) as usize;
                for w in ws.iter_mut() {
                    *w = minw;
                }
                over = shown.split_off(k.min(shown.len()));
                if let Some(p) = over.iter().position(|&i| wins[i].0 == st.active) {
                    let a = over.remove(p);
                    if let Some(last) = shown.pop() {
                        over.insert(0, last);
                    }
                    shown.push(a);
                }
            }
        }
        // the buttons are one Tab stop (the arrows move between them)
        self.focus_group_begin(pid(id, P_TASK, 1));
        let mut x = area[0];
        for &k in &shown {
            let (key, ref title, icon, wf) = wins[k];
            let s = st.state(key);
            let w = ws[k];
            let r = [x, area[1], w, area[3]];
            x += w + gap;
            let bid = pid(id, P_TBTN, key);
            let ev = self.hit_rect(bid, r[0], r[1], r[2], r[3], HB_FOCUS);
            let state = self.wp.ctx.last_state;
            let on = st.active == key && s & DW_MIN == 0;
            let mut bsp = Sp { ev, label: Some(title), icon, value: on as i32, value2: (s & DW_MIN != 0) as i32, flags, ..Sp::default() };
            bsp.place(r);
            sh.taskbar_button(self, &bsp);
            if self.focus_ring(ev) {
                let stl = self.style;
                stl.focus_ring(self, r, 4);
            }
            self.wp.ctx.last_rect = r;
            self.wp.ctx.last_state = state | if on { SS_CHECKED as u32 } else { 0 };
            let name = named(title, s & DW_MIN);
            self.sem(K_BUTTON, bid, Some(&name), s as i32);
            if ev & EV_ACT != 0 {
                if on {
                    if wf & DWF_NOMIN == 0 {
                        st.minimize(key);
                        rt.evs.push(DeskEv::State(key));
                    }
                } else {
                    if s & DW_MIN != 0 {
                        rt.evs.push(DeskEv::State(key));
                    }
                    st.activate(key);
                    rt.focus_kb = self.wp.ctx.focus_visible;
                }
                self.repass = true;
            }
            // its menu at the button (a right click: at the pointer)
            let menu = pid(id, P_TMENU, key);
            if ev & EV_RCLICK != 0 {
                self.popup_open(menu);
                let kb = self.kb_ctx_rect().is_some();
                if kb {
                    self.pp_flag(menu, super::popup::PPF_FOCUS);
                }
                rt.tmenu_point = if kb { 0 } else { menu };
            }
            self.wp.ctx.last_rect = r;
            let fl = if rt.tmenu_point == menu { FL_POINT } else { FL_ABOVE };
            if self.flyout_begin(menu, fl) {
                let rows: [(&'static str, bool); 4] = [
                    (tr("Wiederherstellen"), s & (DW_MAX | DW_MIN | DW_LEFT | DW_RIGHT) != 0),
                    (tr("Minimieren"), wf & DWF_NOMIN == 0 && s & DW_MIN == 0),
                    (tr("Maximieren"), wf & (DWF_NOMAX | DWF_FIXED) == 0 && s & DW_MAX == 0),
                    (tr("Schließen"), wf & DWF_NOCLOSE == 0),
                ];
                for (j, (label, ok)) in rows.into_iter().enumerate() {
                    if j == 3 {
                        self.menu_sep();
                    }
                    if !ok {
                        self.next_flags(ST_DISABLED);
                    }
                    if self.menu_item(pid(id, P_TITEM + j as u32, key), label) & EV_ACT != 0 && ok {
                        match j {
                            0 => st.restore(key),
                            1 => st.minimize(key),
                            2 => st.maximize(key),
                            _ => {}
                        }
                        rt.evs.push(if j == 3 { DeskEv::Close(key) } else { DeskEv::State(key) });
                        self.repass = true;
                    }
                }
            } else if rt.tmenu_point == menu {
                rt.tmenu_point = 0;
            }
            self.flyout_end();
        }
        self.focus_group_end();
        // the windows that do not fit
        if !over.is_empty() {
            let ow = self.dp(OVER_W);
            let r = [area[0] + area[2] - ow, area[1], ow, area[3]];
            let oid = pid(id, P_OVER, 0);
            self.next_rect_px(r[0], r[1], r[2], r[3]);
            self.next_icon(IC_MORE);
            self.next_flags(ST_SUBTLE | ST_ICON_ONLY | ST_COMPACT);
            if self.button(oid, tr("Weitere Fenster")) & EV_ACT != 0 {
                self.popup_open(oid);
                if self.wp.ctx.focus_visible {
                    self.pp_flag(oid, super::popup::PPF_FOCUS);
                }
            }
            if self.flyout_begin(oid, FL_ABOVE | FL_ALIGN_END) {
                for &k in &over {
                    let (key, ref title, icon, _) = wins[k];
                    let s = st.state(key);
                    if icon != 0 {
                        self.next_icon(icon);
                    }
                    if self.menu_item(pid(id, P_OITEM, key), &named(title, s & DW_MIN)) & EV_ACT != 0 {
                        if s & DW_MIN != 0 {
                            rt.evs.push(DeskEv::State(key));
                        }
                        st.activate(key);
                        rt.focus_kb = self.wp.ctx.focus_visible;
                        self.repass = true;
                    }
                }
            }
            self.flyout_end();
        }
        // the keyboard in the taskbar counts as inside the area (Alt+F6 ...), also when the
        // taskbar comes after desk_end
        let focus = self.wp.ctx.focus;
        if focus != 0 && (wins.iter().any(|w| pid(id, P_TBTN, w.0) == focus) || focus == pid(id, P_OVER, 0)) {
            rt.focus_here = true;
            rt.focus_in = true;
        }
        self.dw_put(rt);
        self.box_end();
    }
}
