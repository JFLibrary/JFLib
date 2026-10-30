// dock -- a docking area: the app's panels at the sides of a document area (left, right,
// bottom) in tab groups with splitters, dragged to other places with drop markers, floating in
// tool windows and docked again, hidden, collapsed to a rail; the layout is the app's DockState
//
//   dock_begin(id, &mut st, panels)   the area takes the next place (without next_size it fills
//                                     the box); panels describe every panel each frame
//                                     (DockPanel: id != 0, title, icon, DP_* flags, first place)
//   dock_next(&mut st) -> Some(id)    the next place to fill: DOCK_CENTER (the document area)
//                                     first, then the shown panel of every group, the floating
//                                     windows last; emit its content (a vertical box that clips)
//                                     and call it again until None
//   dock_end(&mut st) -> changed      true when the user changed the layout (save it then)
//   DockState                         the layout, kept by the app: save() -> text, load(text),
//                                     reset, show / hide / toggle / focus / activate, float /
//                                     dock_to, collapse / collapsed, size / set_size, groups /
//                                     floats / hidden_panels, side / visible / front / shown
//
// Places: a side holds tab groups (left / right: one above the other, bottom: side by side)
// with splitters between them and towards the document area; a collapsed side is a rail of
// icon buttons.  Left and right are the start and the end side: the layout is logical, so a
// mirrored (RTL) window shows the left side at the right like everything else.
// Dragging a tab (or the empty part of a tab strip: the whole group) shows drop markers at the
// three edges of the area (a new group on that side) and on the group under the pointer (as a
// tab, before, after it); a release over a tab strip inserts it there; anywhere else -- also
// outside the window -- the panel floats in a tool window at the pointer (WF_TOOL, window ids
// id + 1 .. id + DOCK_FLOAT_MAX).  Tabs of a floating window drag the same way onto the main
// window's markers (the pointer stays with the floating window); its pin button or a double
// click docks it where it came from, its close button hides its panels.  Esc cancels a drag.
// Keyboard: the tab strip of a group is one focus stop (Left / Right / Home / End switch tabs,
// the menu key or Shift+F10 open the group's menu with every move), Ctrl+PgDn / Ctrl+PgUp switch
// tabs while the focus is anywhere in the group, DockState::focus(id) brings a panel forward
// and gives its tab strip the focus (the app binds it to its commands).
// Structural changes (clicks, drops, menu rows) are applied at dock_end, followed by another
// pass; splitters change the sizes at once.  Widget ids are parts of the area id (pid).

use alloc::string::String;
use alloc::vec::Vec;

use super::consts::*;
use super::keys::*;
use super::layout::{dp_at, px2dip_at};
use super::win::Shared;
use super::{Sp, Ui};

/// the sides of a dock area (DockPanel::at, DockState::side); left / right are the start / end
/// side (a mirrored window shows them the other way round)
pub const DOCK_LEFT: u8 = 0;
pub const DOCK_RIGHT: u8 = 1;
pub const DOCK_BOTTOM: u8 = 2;
/// a floating tool window
pub const DOCK_FLOAT: u8 = 3;
/// dock_next: the document area in the middle
pub const DOCK_CENTER: u32 = 0;
/// DockPanel flags: no hide button or menu row (the app shows and hides it)
pub const DP_NOCLOSE: u32 = 1;
/// never floats (a drop outside the markers does nothing)
pub const DP_NOFLOAT: u32 = 2;
/// starts hidden (until DockState::show / focus); later the layout remembers it
pub const DP_HIDDEN: u32 = 4;
/// floating windows of one dock area at most (fixed: each is a window, WIN_MAX)
pub const DOCK_FLOAT_MAX: usize = 6;
/// drop marker kinds (StyleShell::dock_marker, sp.value): the part of the place it fills
pub const DOCK_MK_LEFT: i32 = 0;
pub const DOCK_MK_RIGHT: i32 = 1;
pub const DOCK_MK_TOP: i32 = 2;
pub const DOCK_MK_BOTTOM: i32 = 3;
pub const DOCK_MK_CENTER: i32 = 4;

// metrics (DIP)
const HDR: i32 = 32; // tab strip of a group, unless the style's SM_DOCK_HDR says otherwise
const BTN: i32 = 28; // its buttons (SM_DOCK_BTN)
const GRIP: i32 = 6; // splitters
const RAIL: i32 = 36; // a collapsed side
const SIDE_MIN: i32 = 120;
const CENTER_MIN: i32 = 160;
const GROUP_MIN: i32 = 64;
const MARK: i32 = 36; // a drop marker
const SIDE_DEF: [i32; 3] = [260, 280, 200];
const GROUP_DEF: i32 = 200;
const FLOAT_W: (i32, i32) = (200, 480); // the client size a floated group gets (clamped)
const FLOAT_H: (i32, i32) = (160, 480);
const TAG_USER: u32 = 0x100; // tags of the groups the user made (below: DockPanel groups)
const SIZE_MAX: i32 = 8192;

// id parts (pid)
const P_AREA: u32 = 1;
const P_CENTER: u32 = 2;
const P_CONTENT: u32 = 3;
const P_TAB: u32 = 4;
const P_STRIP: u32 = 5;
const P_MENU: u32 = 6;
const P_BTN1: u32 = 7;
const P_BTN2: u32 = 8;
const P_GRIP: u32 = 9;
const P_GGRIP: u32 = 10;
const P_RAIL: u32 = 11;
const P_MARK: u32 = 12;
const P_MITEM: u32 = 16; // .. + 8

// a widget id of dock area `dock`: part `part` of the panel / group `a`
fn pid(dock: u32, part: u32, a: u32) -> u32 {
    let mut h = dock.wrapping_mul(0x9E37_79B1) ^ part.wrapping_mul(0x85EB_CA6B) ^ a.wrapping_mul(0xC2B2_AE35);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^= h >> 12;
    h | 0x8000_0000
}

// the key of a group: side and tag (floating windows: DOCK_FLOAT and the slot)
fn gkey(side: u8, tag: u32) -> u32 {
    (side as u32) << 24 | (tag & 0x00FF_FFFF)
}

fn inside(r: [i32; 4], p: [i32; 2]) -> bool {
    p[0] >= r[0] && p[0] < r[0] + r[2] && p[1] >= r[1] && p[1] < r[1] + r[3]
}

/// a panel as the app describes it to dock_begin every frame
#[derive(Clone, Copy, Debug)]
pub struct DockPanel<'a> {
    /// != 0 and stable (the saved layout refers to it)
    pub id: u32,
    pub title: &'a str,
    /// IC_* of its tab and of the rail of a collapsed side (0 = none)
    pub icon: u32,
    /// DP_*
    pub flags: u32,
    /// where it goes the first time: DOCK_LEFT / DOCK_RIGHT / DOCK_BOTTOM / DOCK_FLOAT
    pub side: u8,
    /// panels of one side with the same group number start as tabs of one group; the groups
    /// in the order of their numbers
    pub group: u8,
}

impl<'a> DockPanel<'a> {
    /// a panel that starts in group 0 of the left side
    pub const fn new(id: u32, title: &'a str, icon: u32) -> DockPanel<'a> {
        DockPanel { id, title, icon, flags: 0, side: DOCK_LEFT, group: 0 }
    }
    /// its first place: side (DOCK_*) and group number
    pub const fn at(mut self, side: u8, group: u8) -> DockPanel<'a> {
        self.side = side;
        self.group = group;
        self
    }
    /// add DP_* flags
    pub const fn flags(mut self, f: u32) -> DockPanel<'a> {
        self.flags |= f;
        self
    }
}

// ------------------------------------------------------------------ the layout (DockState)

#[derive(Clone, Default, Debug, PartialEq, Eq)]
struct Group {
    /// unique on its side (DockPanel::group for the first groups, from TAG_USER for new ones)
    tag: u32,
    /// share of the side's length (DIP when it was last set; 0 = GROUP_DEF)
    size: i32,
    /// the shown tab
    active: u32,
    panels: Vec<u32>,
}

#[derive(Clone, Default, Debug, PartialEq, Eq)]
struct Side {
    /// width (left / right) or height (bottom) in DIP, 0 = the default
    size: i32,
    collapsed: bool,
    groups: Vec<Group>,
}

#[derive(Clone, Default, Debug, PartialEq, Eq)]
struct Float {
    slot: u8,
    /// x, y: outer top left in DIP from the main window's client origin (physical); client
    /// width and height (DIP) -- a WinRect
    rect: [i32; 4],
    /// where the pin button docks it
    home: u8,
    home_tag: u32,
    group: Group,
}

// a hidden panel and where it comes back (also: where a panel was before it moved)
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
struct Hidden {
    panel: u32,
    /// DOCK_* (DOCK_FLOAT: a floating window at rect)
    side: u8,
    tag: u32,
    pos: u32,
    rect: [i32; 4],
    home: u8,
    home_tag: u32,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Loc {
    None,
    Side(usize, usize, usize),
    Float(usize, usize),
    Hidden(usize),
}

/// the layout of a dock area: kept by the app and passed to dock_begin / dock_next /
/// dock_end.  save() / load() turn it into a few lines of text: `dock 1`, then `side S SIZE
/// COLLAPSED`, `group S TAG SIZE ACTIVE PANEL...`, `float SLOT X Y W H HOME HOMETAG ACTIVE
/// PANEL...`, `hidden PANEL S TAG POS X Y W H HOME HOMETAG` (unknown lines are skipped).
#[derive(Clone, Default, Debug, PartialEq, Eq)]
pub struct DockState {
    sides: [Side; 3],
    floats: Vec<Float>,
    hidden: Vec<Hidden>,
    /// a pending DockState::focus (not saved)
    focus_req: u32,
}

impl DockState {
    /// the empty layout: every panel goes to its DockPanel place in the first frame
    pub fn new() -> DockState {
        DockState::default()
    }

    fn loc(&self, id: u32) -> Loc {
        for (s, sd) in self.sides.iter().enumerate() {
            for (g, gr) in sd.groups.iter().enumerate() {
                if let Some(t) = gr.panels.iter().position(|&p| p == id) {
                    return Loc::Side(s, g, t);
                }
            }
        }
        for (f, fl) in self.floats.iter().enumerate() {
            if let Some(t) = fl.group.panels.iter().position(|&p| p == id) {
                return Loc::Float(f, t);
            }
        }
        match self.hidden.iter().position(|h| h.panel == id) {
            Some(h) => Loc::Hidden(h),
            None => Loc::None,
        }
    }

    // take panel id out of its place -> where it was (None: the layout does not know it); an
    // emptied group or floating window goes away
    fn take(&mut self, id: u32) -> Option<Hidden> {
        fn unlink(g: &mut Group, t: usize, id: u32) {
            g.panels.remove(t);
            if g.active == id {
                let n = g.panels.len();
                g.active = if n == 0 { 0 } else { g.panels[t.min(n - 1)] };
            }
        }
        match self.loc(id) {
            Loc::Side(s, g, t) => {
                let gr = &mut self.sides[s].groups[g];
                let tag = gr.tag;
                unlink(gr, t, id);
                if gr.panels.is_empty() {
                    self.sides[s].groups.remove(g);
                }
                Some(Hidden { panel: id, side: s as u8, tag, pos: t as u32, rect: [0; 4], home: s as u8, home_tag: tag })
            }
            Loc::Float(f, t) => {
                let fl = &mut self.floats[f];
                unlink(&mut fl.group, t, id);
                let h = Hidden { panel: id, side: DOCK_FLOAT, tag: 0, pos: t as u32, rect: fl.rect, home: fl.home, home_tag: fl.home_tag };
                if fl.group.panels.is_empty() {
                    self.floats.remove(f);
                }
                Some(h)
            }
            Loc::Hidden(h) => Some(self.hidden.remove(h)),
            Loc::None => None,
        }
    }

    // a tag no group of side s has (the user's groups: from TAG_USER)
    fn new_tag(&self, s: usize) -> u32 {
        self.sides[s].groups.iter().map(|g| g.tag + 1).max().unwrap_or(0).max(TAG_USER)
    }

    // ids as tabs of the group (s, tag) from pos on (the group is made at the end when there is
    // none); `active` is shown, the side expands
    fn put_tabs(&mut self, s: usize, tag: u32, pos: usize, ids: &[u32], active: u32) {
        let sd = &mut self.sides[s];
        let g = match sd.groups.iter().position(|g| g.tag == tag) {
            Some(g) => g,
            None => {
                sd.groups.push(Group { tag, ..Group::default() });
                sd.groups.len() - 1
            }
        };
        let gr = &mut sd.groups[g];
        let pos = pos.min(gr.panels.len());
        for (k, &p) in ids.iter().enumerate() {
            gr.panels.insert(pos + k, p);
        }
        gr.active = active;
        sd.collapsed = false;
    }

    // ids as a new group of side s: before / after the group with tag `at` (it gives half of
    // its share), else at the end
    fn put_group(&mut self, s: usize, at: Option<(u32, bool)>, ids: &[u32], active: u32) {
        let tag = self.new_tag(s);
        let sd = &mut self.sides[s];
        let mut size = 0;
        let mut i = sd.groups.len();
        if let Some((t, after)) = at {
            if let Some(g) = sd.groups.iter().position(|g| g.tag == t) {
                let half = (if sd.groups[g].size > 0 { sd.groups[g].size } else { GROUP_DEF } / 2).max(1);
                sd.groups[g].size = half;
                size = half;
                i = g + after as usize;
            }
        }
        sd.groups.insert(i, Group { tag, size, active, panels: ids.to_vec() });
        sd.collapsed = false;
    }

    // ids in a new floating window -> false when every slot is taken
    fn put_float(&mut self, rect: [i32; 4], home: u8, home_tag: u32, ids: &[u32], active: u32) -> bool {
        let Some(slot) = (0..DOCK_FLOAT_MAX as u8).find(|k| !self.floats.iter().any(|f| f.slot == *k)) else {
            return false;
        };
        let group = Group { tag: 0, size: 0, active, panels: ids.to_vec() };
        self.floats.push(Float { slot, rect, home: home.min(DOCK_BOTTOM), home_tag, group });
        true
    }

    // a place taken with take: the panel goes back there
    fn restore(&mut self, h: Hidden) {
        let id = h.panel;
        if h.side < DOCK_FLOAT {
            self.put_tabs(h.side as usize, h.tag, h.pos as usize, &[id], id);
        } else if !self.put_float(h.rect, h.home, h.home_tag, &[id], id) {
            self.put_tabs(h.home.min(DOCK_BOTTOM) as usize, h.home_tag, usize::MAX, &[id], id);
        }
    }

    fn cascade(&self) -> [i32; 4] {
        let k = self.floats.len() as i32;
        [40 + 32 * k, 80 + 32 * k, 280, 320]
    }

    // panels the layout does not know yet go to their DockPanel places
    fn adopt(&mut self, panels: &[DockPanel]) {
        for p in panels {
            if p.id == 0 || self.loc(p.id) != Loc::None {
                continue;
            }
            let tag = p.group as u32;
            let float = p.side >= DOCK_FLOAT;
            let home = if float { DOCK_RIGHT } else { p.side };
            let rect = self.cascade();
            if p.flags & DP_HIDDEN != 0 {
                let side = if float { DOCK_FLOAT } else { home };
                self.hidden.push(Hidden { panel: p.id, side, tag, pos: u32::MAX, rect, home, home_tag: tag });
                continue;
            }
            if float && self.put_float(rect, home, tag, &[p.id], p.id) {
                continue;
            }
            let sd = &mut self.sides[home as usize];
            match sd.groups.iter().position(|g| g.tag == tag) {
                Some(g) => {
                    let gr = &mut sd.groups[g];
                    gr.panels.push(p.id);
                    if gr.active == 0 {
                        gr.active = p.id;
                    }
                }
                None => {
                    let i = sd.groups.iter().position(|g| g.tag > tag).unwrap_or(sd.groups.len());
                    sd.groups.insert(i, Group { tag, size: 0, active: p.id, panels: alloc::vec![p.id] });
                }
            }
        }
    }

    /// forget the layout: every panel goes to its DockPanel place in the next frame
    pub fn reset(&mut self) {
        *self = DockState::default();
    }

    /// show a hidden panel where it was, as the shown tab of its group
    pub fn show(&mut self, id: u32) {
        if let Loc::Hidden(_) = self.loc(id) {
            if let Some(h) = self.take(id) {
                self.restore(h);
            }
        }
        self.activate(id);
    }

    /// hide a panel (show brings it back to the same place)
    pub fn hide(&mut self, id: u32) {
        if matches!(self.loc(id), Loc::Side(..) | Loc::Float(..)) {
            if let Some(h) = self.take(id) {
                self.hidden.push(h);
            }
        }
    }

    /// show a hidden panel, hide a visible one
    pub fn toggle(&mut self, id: u32) {
        if self.visible(id) {
            self.hide(id);
        } else {
            self.show(id);
        }
    }

    /// show the panel as the shown tab (a collapsed side expands) and give its tab strip the
    /// keyboard focus in the next pass (a floating window comes to the front)
    pub fn focus(&mut self, id: u32) {
        self.show(id);
        if self.visible(id) {
            self.focus_req = id;
        }
    }

    /// make the panel the shown tab of its group (a collapsed side expands)
    pub fn activate(&mut self, id: u32) {
        match self.loc(id) {
            Loc::Side(s, g, _) => {
                self.sides[s].groups[g].active = id;
                self.sides[s].collapsed = false;
            }
            Loc::Float(f, _) => self.floats[f].group.active = id,
            _ => {}
        }
    }

    /// float the panel in a tool window: rect = x, y (DIP from the main window's client
    /// origin), client width, height; None: cascaded from the top left -> false when every
    /// floating window of the area is in use
    pub fn float(&mut self, id: u32, rect: Option<[i32; 4]>) -> bool {
        if let Loc::Float(f, _) = self.loc(id) {
            if self.floats[f].group.panels.len() == 1 {
                if let Some(r) = rect {
                    self.floats[f].rect = r;
                }
                return true;
            }
        }
        let r = rect.unwrap_or_else(|| self.cascade());
        let Some(h) = self.take(id) else { return false };
        let (home, home_tag) = if h.side < DOCK_FLOAT { (h.side, h.tag) } else { (h.home, h.home_tag) };
        if self.put_float(r, home, home_tag, &[id], id) {
            return true;
        }
        self.restore(h);
        false
    }

    /// dock the panel to a side, as a new group at its end
    pub fn dock_to(&mut self, id: u32, side: u8) {
        if self.take(id).is_some() {
            self.put_group(side.min(DOCK_BOTTOM) as usize, None, &[id], id);
        }
    }

    // every panel of floating window `slot` back to where it came from
    fn dock_home(&mut self, slot: u8) {
        let Some(f) = self.floats.iter().position(|f| f.slot == slot) else { return };
        let fl = self.floats.remove(f);
        let s = fl.home.min(DOCK_BOTTOM) as usize;
        self.put_tabs(s, fl.home_tag, usize::MAX, &fl.group.panels, fl.group.active);
    }

    // every panel of floating window `slot` hidden (its close button)
    fn hide_float(&mut self, slot: u8) {
        if let Some(f) = self.floats.iter().position(|f| f.slot == slot) {
            for p in self.floats[f].group.panels.clone() {
                self.hide(p);
            }
        }
    }

    /// collapse a side to a rail of icon buttons, or expand it
    pub fn collapse(&mut self, side: u8, on: bool) {
        if let Some(s) = self.sides.get_mut(side as usize) {
            s.collapsed = on;
        }
    }

    /// the side is collapsed
    pub fn collapsed(&self, side: u8) -> bool {
        self.sides.get(side as usize).is_some_and(|s| s.collapsed)
    }

    /// the width (left / right) or the height (bottom) of a side in DIP
    pub fn size(&self, side: u8) -> i32 {
        let s = side.min(DOCK_BOTTOM) as usize;
        if self.sides[s].size > 0 { self.sides[s].size } else { SIDE_DEF[s] }
    }

    /// set the width / height of a side (DIP)
    pub fn set_size(&mut self, side: u8, dip: i32) {
        if let Some(s) = self.sides.get_mut(side as usize) {
            s.size = dip.clamp(SIDE_MIN, SIZE_MAX);
        }
    }

    /// the tab groups of a side from start to end, each with its panels in tab order
    pub fn groups(&self, side: u8) -> Vec<Vec<u32>> {
        self.sides.get(side as usize).map_or(Vec::new(), |s| s.groups.iter().map(|g| g.panels.clone()).collect())
    }

    /// the panels of every floating window
    pub fn floats(&self) -> Vec<Vec<u32>> {
        self.floats.iter().map(|f| f.group.panels.clone()).collect()
    }

    /// the hidden panels
    pub fn hidden_panels(&self) -> Vec<u32> {
        self.hidden.iter().map(|h| h.panel).collect()
    }

    /// where the panel is: DOCK_LEFT / DOCK_RIGHT / DOCK_BOTTOM / DOCK_FLOAT (None: hidden or
    /// unknown)
    pub fn side(&self, id: u32) -> Option<u8> {
        match self.loc(id) {
            Loc::Side(s, _, _) => Some(s as u8),
            Loc::Float(..) => Some(DOCK_FLOAT),
            _ => None,
        }
    }

    /// docked or floating (not hidden)
    pub fn visible(&self, id: u32) -> bool {
        self.side(id).is_some()
    }

    /// the shown tab of its group (also on a collapsed side)
    pub fn front(&self, id: u32) -> bool {
        match self.loc(id) {
            Loc::Side(s, g, _) => self.sides[s].groups[g].active == id,
            Loc::Float(f, _) => self.floats[f].group.active == id,
            _ => false,
        }
    }

    /// its content is on the screen: the front tab, on a side that is not collapsed (or
    /// floating)
    pub fn shown(&self, id: u32) -> bool {
        self.front(id) && !matches!(self.loc(id), Loc::Side(s, _, _) if self.sides[s].collapsed)
    }

    /// the layout as text (see DockState)
    pub fn save(&self) -> String {
        use core::fmt::Write;
        let mut o = String::from("dock 1\n");
        for (s, sd) in self.sides.iter().enumerate() {
            let _ = writeln!(o, "side {} {} {}", s, sd.size, sd.collapsed as u8);
        }
        let list = |o: &mut String, ids: &[u32]| {
            for p in ids {
                let _ = write!(o, " {}", p);
            }
            o.push('\n');
        };
        for (s, sd) in self.sides.iter().enumerate() {
            for g in &sd.groups {
                let _ = write!(o, "group {} {} {} {}", s, g.tag, g.size, g.active);
                list(&mut o, &g.panels);
            }
        }
        for f in &self.floats {
            let r = f.rect;
            let _ = write!(o, "float {} {} {} {} {} {} {} {}", f.slot, r[0], r[1], r[2], r[3], f.home, f.home_tag, f.group.active);
            list(&mut o, &f.group.panels);
        }
        for h in &self.hidden {
            let r = h.rect;
            let _ = writeln!(o, "hidden {} {} {} {} {} {} {} {} {} {}", h.panel, h.side, h.tag, h.pos, r[0], r[1], r[2], r[3], h.home, h.home_tag);
        }
        o
    }

    /// a layout saved with save -> None when the text is no dock layout (version 1)
    pub fn load(text: &str) -> Option<DockState> {
        let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#'));
        let mut w = lines.next()?.split_whitespace();
        if w.next()? != "dock" || w.next()? != "1" {
            return None;
        }
        let mut st = DockState::default();
        let mut seen: Vec<u32> = Vec::new();
        for l in lines {
            let mut w = l.split_whitespace();
            let kind = w.next().unwrap_or("");
            let v: Vec<i64> = w.map(|x| x.parse::<i64>().unwrap_or(i64::MIN)).collect();
            if v.contains(&i64::MIN) {
                continue;
            }
            let num = |i: usize| v.get(i).copied().unwrap_or(0);
            let int = |i: usize| num(i).clamp(-(SIZE_MAX as i64) * 4, SIZE_MAX as i64 * 4) as i32;
            let id = |i: usize| num(i).clamp(0, u32::MAX as i64) as u32;
            // the panels of v[i..j] that no earlier line had
            let mut fresh = |i: usize, j: usize| -> Vec<u32> {
                let mut out = Vec::new();
                for k in i..j.min(v.len()) {
                    let p = id(k);
                    if p != 0 && !seen.contains(&p) {
                        seen.push(p);
                        out.push(p);
                    }
                }
                out
            };
            match kind {
                "side" if v.len() >= 3 && (0..3).contains(&num(0)) => {
                    let sd = &mut st.sides[num(0) as usize];
                    sd.size = int(1).clamp(0, SIZE_MAX);
                    sd.collapsed = num(2) != 0;
                }
                "group" if v.len() >= 5 && (0..3).contains(&num(0)) => {
                    let s = num(0) as usize;
                    let mut tag = id(1) & 0x00FF_FFFF;
                    let panels = fresh(4, v.len());
                    if panels.is_empty() {
                        continue;
                    }
                    if st.sides[s].groups.iter().any(|g| g.tag == tag) {
                        tag = st.new_tag(s);
                    }
                    let active = if panels.contains(&id(3)) { id(3) } else { panels[0] };
                    st.sides[s].groups.push(Group { tag, size: int(2).clamp(0, SIZE_MAX), active, panels });
                }
                "float" if v.len() >= 9 => {
                    let panels = fresh(8, v.len());
                    if panels.is_empty() || st.floats.len() >= DOCK_FLOAT_MAX {
                        continue;
                    }
                    let mut slot = num(0).clamp(0, DOCK_FLOAT_MAX as i64 - 1) as u8;
                    if st.floats.iter().any(|f| f.slot == slot) {
                        slot = (0..DOCK_FLOAT_MAX as u8).find(|k| !st.floats.iter().any(|f| f.slot == *k)).unwrap_or(0);
                    }
                    let rect = [int(1), int(2), int(3).clamp(1, SIZE_MAX), int(4).clamp(1, SIZE_MAX)];
                    let active = if panels.contains(&id(7)) { id(7) } else { panels[0] };
                    let home = num(5).clamp(0, 2) as u8;
                    st.floats.push(Float { slot, rect, home, home_tag: id(6) & 0x00FF_FFFF, group: Group { tag: 0, size: 0, active, panels } });
                }
                "hidden" if v.len() >= 10 => {
                    let Some(&panel) = fresh(0, 1).first() else { continue };
                    let rect = [int(4), int(5), int(6).clamp(1, SIZE_MAX), int(7).clamp(1, SIZE_MAX)];
                    st.hidden.push(Hidden {
                        panel,
                        side: num(1).clamp(0, 3) as u8,
                        tag: id(2) & 0x00FF_FFFF,
                        pos: id(3),
                        rect,
                        home: num(8).clamp(0, 2) as u8,
                        home_tag: id(9) & 0x00FF_FFFF,
                    });
                }
                _ => {}
            }
        }
        Some(st)
    }
}

// ------------------------------------------------------------------ the runtime part

// the frame's description of a panel
struct PInfo {
    id: u32,
    title: String,
    icon: u32,
    flags: u32,
}

// a docked group as laid out in this pass (main window px, logical)
#[derive(Clone, Default)]
struct Geo {
    side: u8,
    tag: u32,
    rect: [i32; 4],
    hdr: [i32; 4],
    body: [i32; 4],
    /// the panels the app describes this frame, in tab order, and their tabs
    panels: Vec<u32>,
    tabs: Vec<[i32; 4]>,
    active: u32,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Target {
    None,
    /// a new group at the end of a side
    Side(u8),
    /// tabs of group (side, tag) at an index (usize::MAX: the end)
    Tab(u8, u32, usize),
    /// a new group before (false) / after (true) group (side, tag)
    Split(u8, u32, bool),
    /// a floating window at the pointer
    Float,
}

struct Mark {
    kind: i32,
    rect: [i32; 4],
    to: Target,
    label: &'static str,
}

// a panel (or a group) being dragged
#[derive(Clone)]
struct Drag {
    panels: Vec<u32>,
    active: u32,
    /// the group it comes from: (side, tag), a floating window (DOCK_FLOAT, slot)
    src: (u8, u32),
    /// the window and the widget that hold the pointer
    win: u32,
    wid: u32,
    /// the pointer's offset from the start corner of the source's tab strip (DIP)
    grab: [i32; 2],
    /// the client size a floating window of it gets (DIP)
    size: [i32; 2],
    /// the pointer in the main window (logical px)
    at: [i32; 2],
    seen: u32,
}

enum Act {
    Drop(Drag),
    Float(u32, [i32; 4]),
    Home(u8),
    ToSide(u32, u8),
    Hide(u32),
    Collapse(u8, bool),
    Activate(u32),
}

#[derive(Clone, Copy)]
enum Slot {
    Center,
    Group(usize),
    Float(usize),
}

// the library's side of a floating window: its open flag and placement
#[derive(Default)]
struct WinSlot {
    open: Shared<i32>,
    rect: Shared<WinRect>,
    applied: [i32; 4],
    used: bool,
}

/// one dock area's state between the calls (and the passes)
#[derive(Default)]
pub(crate) struct Rt {
    id: u32,
    panels: Vec<PInfo>,
    area: [i32; 4],
    center: [i32; 4],
    /// per side: 0 absent, 1 shown, 2 collapsed (a rail); its rectangle and its splitter
    on: [u8; 3],
    side: [[i32; 4]; 3],
    grip: [[i32; 4]; 3],
    groups: Vec<Geo>,
    slots: Vec<Slot>,
    next: usize,
    /// what dock_next opened: 0 nothing, 1 a box (center / group), 2 a floating window
    open: u8,
    /// the open group / floating window (gkey) and its strip, the hit count before its content
    open_key: u32,
    open_panels: Vec<u32>,
    open_active: u32,
    hits0: usize,
    drag: Option<Drag>,
    acts: Vec<Act>,
    wins: Vec<WinSlot>,
    /// groups (gkey) that had the keyboard focus in the last pass / this pass
    focus_in: Vec<u32>,
    focus_now: Vec<u32>,
    /// the menu opened by a right click (anchored to the pointer)
    menu_point: u32,
    changed: bool,
}

impl Rt {
    fn info(&self, id: u32) -> Option<&PInfo> {
        self.panels.iter().find(|p| p.id == id)
    }
    fn present(&self, id: u32) -> bool {
        self.panels.iter().any(|p| p.id == id)
    }
    fn title(&self, id: u32) -> String {
        self.info(id).map_or(String::new(), |p| p.title.clone())
    }
    fn flags(&self, id: u32) -> u32 {
        self.info(id).map_or(0, |p| p.flags)
    }
    // the group's panels the app describes, and the one shown
    fn shown(&self, g: &Group) -> (Vec<u32>, u32) {
        let v: Vec<u32> = g.panels.iter().copied().filter(|&p| self.present(p)).collect();
        let a = if v.contains(&g.active) { g.active } else { v.first().copied().unwrap_or(0) };
        (v, a)
    }
}

/// the dock areas of the app (Mods)
#[derive(Default)]
pub(crate) struct S {
    docks: Vec<Rt>,
    /// the areas between dock_begin and dock_end
    cur: Vec<u32>,
}

// a library text (German source; i18n::tr)
fn tr(s: &'static str) -> &'static str {
    super::i18n::tr(s)
}

impl Ui {
    fn dk_take(&mut self, id: u32) -> Rt {
        let d = &mut self.m.dock.docks;
        match d.iter().position(|r| r.id == id) {
            Some(i) => d.swap_remove(i),
            None => Rt { id, ..Rt::default() },
        }
    }

    fn dk_put(&mut self, rt: Rt) {
        self.m.dock.docks.push(rt);
    }

    // a side's name as the user sees it (mirrored windows show left at the right)
    fn dk_side_label(&self, s: u8) -> &'static str {
        let s = if self.rtl && s < DOCK_BOTTOM { s ^ 1 } else { s };
        match s {
            DOCK_LEFT => tr("Links andocken"),
            DOCK_RIGHT => tr("Rechts andocken"),
            _ => tr("Unten andocken"),
        }
    }

    // the style's SM_DOCK_* value in px; def (DIP) when the style gives none
    fn dk_metric(&self, id: u32, def: i32) -> i32 {
        let v = self.style.metric(id);
        self.dp(if v > 0 { v } else { def })
    }

    // vk with exactly these modifiers pressed this frame (consumed; a text box keeps its keys)
    fn dk_key(&mut self, vk: u32, mods: u32) -> bool {
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

    // a point of the live window (logical px) in the main window (logical px)
    fn dk_main_point(&self, x: i32, y: i32) -> [i32; 2] {
        let win = self.uiw_cur;
        if win == 0 {
            return [x, y];
        }
        let px = if self.rtl { self.wp.rt.fb.w - 1 - x } else { x };
        let dpi = self.wp.rt.dpi;
        let (dx, dy) = self.uiw_desk_pos(win);
        let (mx, my) = (dx + px2dip_at(px, dpi), dy + px2dip_at(y, dpi));
        let mdpi = self.win_dpi(0);
        let (mpx, mpy) = (dp_at(mx, mdpi), dp_at(my, mdpi));
        let w0 = self.win_fb(0).w;
        [if self.rtl { w0 - 1 - mpx } else { mpx }, mpy]
    }

    // a logical rectangle of the main window as a floating window's WinRect values (DIP,
    // physical), moved by (dx, dy) DIP and clamped to the floating sizes
    fn dk_float_near(&self, r: [i32; 4], dx: i32, dy: i32) -> [i32; 4] {
        let p = self.phys_rect(r);
        let w = self.px2dip(r[2]).clamp(FLOAT_W.0, FLOAT_W.1);
        let h = self.px2dip(r[3]).clamp(FLOAT_H.0, FLOAT_H.1);
        [self.px2dip(p[0]) + dx, self.px2dip(p[1]) + dy, w, h]
    }

    // ------------------------------------------------------------------ app API

    /// dock_begin(id, &mut st, panels): open the dock area (always pair with dock_end; fill it
    /// with dock_next).  It takes the next place of the box (the whole rest when no size was
    /// set); panels: every panel the app has, each frame (titles and icons may change).
    #[cfg_attr(jfgui_dev, track_caller)]
    pub fn dock_begin(&mut self, id: u32, st: &mut DockState, panels: &[DockPanel]) {
        let _src = self.src_scope();
        let mut rt = self.dk_take(id);
        let cur = &mut self.m.dock.cur;
        cur.retain(|&c| c != id); // (a dock_end the app left out)
        cur.push(id);
        rt.panels = panels.iter().filter(|p| p.id != 0).map(|p| PInfo { id: p.id, title: String::from(p.title), icon: p.icon, flags: p.flags }).collect();
        st.adopt(panels);
        for k in 0..rt.wins.len() {
            let w = &mut rt.wins[k];
            if w.used && w.open.get() == 0 {
                // the user closed the floating window: its panels are hidden
                w.used = false;
                w.open.set(1);
                st.hide_float(k as u8);
                rt.changed = true;
            }
            if !st.floats.iter().any(|f| f.slot as usize == k) {
                w.applied = [i32::MIN; 4]; // a later window in this slot gets its own placement
            }
        }
        // a drag whose widget went away (or was cancelled by Esc in another window)
        if rt.drag.as_ref().is_some_and(|d| self.frame.wrapping_sub(d.seen) > 2) {
            rt.drag = None;
        }
        if rt.drag.as_ref().is_some_and(|d| d.win != 0) && self.key(VK_ESCAPE) {
            rt.drag = None;
            self.repass = true;
        }
        let l = &self.wp.lay;
        if l.next_w == 0 && l.next_h == 0 && l.next_rect_set == 0 {
            self.next_size(SIZE_FILL, SIZE_FILL);
        }
        self.box_begin(pid(id, P_AREA, 0), BOX_V | BOX_FIXED, 0);
        rt.area = self.box_rect();
        self.dk_layout(&mut rt, st);
        self.dk_emit(&mut rt, st);
        // what dock_next yields: the document area, the groups, the floating windows
        rt.slots.clear();
        rt.slots.push(Slot::Center);
        for g in 0..rt.groups.len() {
            rt.slots.push(Slot::Group(g));
        }
        for f in 0..st.floats.len() {
            if st.floats[f].group.panels.iter().any(|&p| rt.present(p)) {
                rt.slots.push(Slot::Float(f));
            }
        }
        rt.next = 0;
        rt.open = 0;
        self.dk_put(rt);
    }

    /// dock_next(&mut st) -> the next place to fill (None: done): DOCK_CENTER, then the id of
    /// a shown panel; its content goes into a vertical box (it clips) until the next call
    #[cfg_attr(jfgui_dev, track_caller)]
    pub fn dock_next(&mut self, st: &mut DockState) -> Option<u32> {
        let _src = self.src_scope();
        let id = *self.m.dock.cur.last()?;
        let mut rt = self.dk_take(id);
        self.dk_close(&mut rt);
        let mut out = None;
        while out.is_none() && rt.next < rt.slots.len() {
            let s = rt.slots[rt.next];
            rt.next += 1;
            match s {
                Slot::Center => {
                    let c = rt.center;
                    self.next_rect_px(c[0], c[1], c[2], c[3]);
                    self.box_begin(pid(id, P_CENTER, 0), BOX_V | BOX_CLIP | BOX_FIXED, 0);
                    self.focus_pane_begin(pid(id, P_CENTER, 0)); // F6 (kbnav.rs)
                    rt.open = 1;
                    rt.open_key = u32::MAX;
                    out = Some(DOCK_CENTER);
                }
                Slot::Group(g) => {
                    let geo = rt.groups[g].clone();
                    let b = geo.body;
                    self.next_rect_px(b[0], b[1], b[2], b[3]);
                    self.box_begin(pid(id, P_CONTENT, geo.active), BOX_V | BOX_CLIP | BOX_FIXED, 0);
                    self.focus_pane_begin(pid(id, P_STRIP, gkey(geo.side, geo.tag))); // with its tab strip
                    rt.open = 1;
                    rt.open_key = gkey(geo.side, geo.tag);
                    rt.open_panels = geo.panels.clone();
                    rt.open_active = geo.active;
                    rt.hits0 = self.wp.ctx.hits[self.wp.ctx.hit_cur].len();
                    out = Some(geo.active);
                }
                Slot::Float(f) => out = self.dk_float_open(&mut rt, st, f),
            }
        }
        self.dk_put(rt);
        out
    }

    /// dock_end(&mut st) -> the user changed the layout in this pass (store st.save() then)
    #[cfg_attr(jfgui_dev, track_caller)]
    pub fn dock_end(&mut self, st: &mut DockState) -> bool {
        let _src = self.src_scope();
        let Some(id) = self.m.dock.cur.pop() else { return false };
        let mut rt = self.dk_take(id);
        self.dk_close(&mut rt);
        self.dk_overlay(&rt, st);
        let acts = core::mem::take(&mut rt.acts);
        for a in acts {
            self.dk_apply(&mut rt, st, a);
        }
        self.box_end(); // the area
        rt.focus_in = core::mem::take(&mut rt.focus_now);
        let changed = core::mem::take(&mut rt.changed);
        self.dk_put(rt);
        changed
    }

    // ------------------------------------------------------------------ layout

    // the sides, their groups and the document area of this pass
    fn dk_layout(&mut self, rt: &mut Rt, st: &DockState) {
        let a = rt.area;
        let g = self.dp(GRIP);
        let rail = self.dp(RAIL);
        for s in 0..3 {
            let any = st.sides[s].groups.iter().any(|gr| gr.panels.iter().any(|&p| rt.present(p)));
            rt.on[s] = if !any { 0 } else if st.sides[s].collapsed { 2 } else { 1 };
        }
        let want = |s: usize| self.dp(st.size(s as u8));
        let gw = |s: usize| if rt.on[s] == 1 { g } else { 0 };
        // left and right: what the document area leaves them
        let mut w = [0i32; 3];
        let fixed: i32 = (0..2).filter(|&s| rt.on[s] == 2).map(|_| rail).sum();
        let room = (a[2] - gw(0) - gw(1) - fixed - self.dp(CENTER_MIN)).max(0);
        for s in 0..2 {
            w[s] = match rt.on[s] {
                1 => want(s),
                2 => rail,
                _ => 0,
            };
        }
        let (e0, e1) = (if rt.on[0] == 1 { w[0] } else { 0 }, if rt.on[1] == 1 { w[1] } else { 0 });
        if e0 + e1 > room {
            if e0 > 0 && e1 > 0 {
                w[0] = (e0 as i64 * room as i64 / (e0 + e1) as i64) as i32;
                w[1] = room - w[0];
            } else if e0 > 0 {
                w[0] = room;
            } else {
                w[1] = room;
            }
        }
        w[2] = match rt.on[2] {
            1 => want(2).min((a[3] - g - self.dp(CENTER_MIN) / 2).max(0)),
            2 => rail,
            _ => 0,
        };
        let (x0, y0, x1, y1) = (a[0], a[1], a[0] + a[2], a[1] + a[3]);
        rt.side[0] = [x0, y0, w[0], a[3]];
        rt.grip[0] = [x0 + w[0], y0, gw(0), a[3]];
        rt.side[1] = [x1 - w[1], y0, w[1], a[3]];
        rt.grip[1] = [x1 - w[1] - gw(1), y0, gw(1), a[3]];
        let cx0 = x0 + w[0] + gw(0);
        let cw = (x1 - w[1] - gw(1) - cx0).max(0);
        rt.side[2] = [cx0, y1 - w[2], cw, w[2]];
        rt.grip[2] = [cx0, y1 - w[2] - gw(2), cw, gw(2)];
        rt.center = [cx0, y0, cw, (y1 - w[2] - gw(2) - y0).max(0)];
        // the groups of the shown sides along them
        rt.groups.clear();
        let hh = self.dk_metric(SM_DOCK_HDR, HDR);
        for s in 0..3 {
            if rt.on[s] != 1 {
                continue;
            }
            let list: Vec<(&Group, Vec<u32>, u32)> = st.sides[s]
                .groups
                .iter()
                .filter_map(|gr| {
                    let (v, act) = rt.shown(gr);
                    if v.is_empty() { None } else { Some((gr, v, act)) }
                })
                .collect();
            let n = list.len() as i32;
            let r = rt.side[s];
            let ax = if s == DOCK_BOTTOM as usize { 0 } else { 1 };
            let avail = (r[2 + ax] - g * (n - 1)).max(0);
            let sum: i64 = list.iter().map(|(gr, _, _)| if gr.size > 0 { gr.size } else { GROUP_DEF } as i64).sum();
            let mut pos = r[ax];
            let mut used = 0;
            for (k, (gr, v, act)) in list.iter().enumerate() {
                let share = if gr.size > 0 { gr.size } else { GROUP_DEF } as i64;
                let len = if k as i32 == n - 1 { avail - used } else { (avail as i64 * share / sum.max(1)) as i32 };
                used += len;
                let mut rc = r;
                rc[ax] = pos;
                rc[2 + ax] = len;
                pos += len + g;
                let h = hh.min(rc[3]);
                rt.groups.push(Geo {
                    side: s as u8,
                    tag: gr.tag,
                    rect: rc,
                    hdr: [rc[0], rc[1], rc[2], h],
                    body: [rc[0], rc[1] + h, rc[2], rc[3] - h],
                    panels: v.clone(),
                    tabs: Vec::new(),
                    active: *act,
                });
            }
        }
    }

    // ------------------------------------------------------------------ the parts

    // splitters, rails, the groups' tab strips
    fn dk_emit(&mut self, rt: &mut Rt, st: &mut DockState) {
        let id = rt.id;
        let sh = self.style.shell();
        let g = self.dp(GRIP);
        // splitters between the sides and the document area
        for s in 0..3u8 {
            if rt.on[s as usize] != 1 {
                continue;
            }
            let r = rt.grip[s as usize];
            let horiz = s == DOCK_BOTTOM;
            let cur = if horiz { CUR_SIZE_NS } else { CUR_SIZE_WE };
            let ev = self.hit_rect(pid(id, P_GRIP, s as u32), r[0], r[1], r[2], r[3], HB_SPLITTER | cur << HB_CURSOR_SHIFT);
            // the keyboard (F8 reaches the grip, kbnav.rs): the grip moves with the arrows
            let d = self.kb_grip(ev, !horiz);
            if d != 0 {
                let v = match s {
                    DOCK_LEFT => st.size(s).saturating_add(d),
                    _ => st.size(s).saturating_sub(d),
                }
                .clamp(SIDE_MIN, SIZE_MAX);
                if v != st.size(s) {
                    st.sides[s as usize].size = v;
                    rt.changed = true;
                    self.repass = true;
                }
            }
            if ev & EV_HELD != 0 {
                let (mx, my) = self.mouse();
                let a = rt.area;
                let px = match s {
                    DOCK_LEFT => mx - a[0],
                    DOCK_RIGHT => a[0] + a[2] - mx,
                    _ => a[1] + a[3] - my,
                } - (g >> 1);
                let v = self.px2dip(px).clamp(SIDE_MIN, SIZE_MAX);
                if v != st.size(s) {
                    st.sides[s as usize].size = v;
                    rt.changed = true;
                    self.repass = true;
                }
            }
            let mut sp = Sp { ev, flags: horiz as u32, ..Sp::default() };
            sp.place(r);
            sh.split_grip(self, &sp);
            if self.focus_ring(ev) {
                let stl = self.style;
                stl.focus_ring(self, r, 0);
            }
            // the record: the side's size (DIP)
            let size = st.size(s);
            let rec = self.sem(K_SLIDER, pid(id, P_GRIP, s as u32), Some(tr("Teiler")), size);
            rec.min = SIDE_MIN;
            rec.max = SIZE_MAX;
        }
        // collapsed sides: a rail with a button per panel
        for s in 0..3u8 {
            if rt.on[s as usize] != 2 {
                continue;
            }
            let r = rt.side[s as usize];
            let mut sp = Sp { flags: s as u32, ..Sp::default() };
            sp.place(r);
            sh.dock_rail(self, &sp);
            let b = self.dp(32);
            let m = (self.dp(RAIL) - b) >> 1;
            let mut k = 0;
            for gr in &st.sides[s as usize].groups {
                for &p in &gr.panels {
                    let Some(info) = rt.info(p) else { continue };
                    let (title, icon) = (info.title.clone(), info.icon);
                    let off = m + k * (b + self.dp(4));
                    if s == DOCK_BOTTOM {
                        self.next_rect_px(r[0] + off, r[1] + m, b, b);
                    } else {
                        self.next_rect_px(r[0] + m, r[1] + off, b, b);
                    }
                    self.next_icon(if icon != 0 { icon } else { IC_DOCUMENT });
                    self.next_flags(ST_SUBTLE | ST_ICON_ONLY);
                    if self.button(pid(id, P_RAIL, p), &title) & EV_ACT != 0 {
                        rt.acts.push(Act::Activate(p));
                    }
                    k += 1;
                }
            }
        }
        // the groups (each tab strip in its group's pane: F6)
        for gi in 0..rt.groups.len() {
            let geo = rt.groups[gi].clone();
            self.focus_pane_begin(pid(id, P_STRIP, gkey(geo.side, geo.tag)));
            let tabs = self.dk_header(rt, st, geo.side, geo.tag, geo.rect, geo.hdr, &geo.panels, geo.active);
            self.focus_pane_end();
            rt.groups[gi].tabs = tabs;
        }
        // splitters between the groups of a side
        for gi in 1..rt.groups.len() {
            let (a, b) = (rt.groups[gi - 1].clone(), rt.groups[gi].clone());
            if a.side != b.side {
                continue;
            }
            let ax = if a.side == DOCK_BOTTOM { 0 } else { 1 };
            let mut r = a.rect;
            r[ax] = a.rect[ax] + a.rect[2 + ax];
            r[2 + ax] = g;
            let cur = if ax == 1 { CUR_SIZE_NS } else { CUR_SIZE_WE };
            let ev = self.hit_rect(pid(id, P_GGRIP, gkey(a.side, a.tag)), r[0], r[1], r[2], r[3], HB_SPLITTER | cur << HB_CURSOR_SHIFT);
            // the first group's new length (px): the keyboard (F8, arrows) or the pointer
            let kd = self.kb_grip(ev, ax == 0).clamp(-100_000, 100_000);
            let want = if kd != 0 {
                Some(a.rect[2 + ax] + self.dp(kd))
            } else if ev & EV_HELD != 0 {
                let (mx, my) = self.mouse();
                let m = if ax == 1 { my } else { mx };
                Some(m - a.rect[ax] - (g >> 1))
            } else {
                None
            };
            if let Some(w) = want {
                let pair = a.rect[2 + ax] + g + b.rect[2 + ax];
                let min = self.dp(GROUP_MIN);
                if pair - g >= 2 * min {
                    let na = w.clamp(min, pair - g - min);
                    if na != a.rect[2 + ax] {
                        // every group of the side keeps its size in DIP, these two share theirs anew
                        let side = a.side as usize;
                        for geo in rt.groups.iter().filter(|x| x.side == a.side) {
                            let len = if geo.tag == a.tag {
                                na
                            } else if geo.tag == b.tag {
                                pair - g - na
                            } else {
                                geo.rect[2 + ax]
                            };
                            if let Some(gr) = st.sides[side].groups.iter_mut().find(|x| x.tag == geo.tag) {
                                gr.size = px2dip_at(len, self.wp.rt.dpi).max(1);
                            }
                        }
                        rt.changed = true;
                        self.repass = true;
                    }
                }
            }
            let mut sp = Sp { ev, flags: (ax == 1) as u32, ..Sp::default() };
            sp.place(r);
            sh.split_grip(self, &sp);
            if self.focus_ring(ev) {
                let stl = self.style;
                stl.focus_ring(self, r, 0);
            }
            // the record: the first group's length of the two (DIP)
            let len = self.px2dip(a.rect[2 + ax]);
            let pair = self.px2dip(a.rect[2 + ax] + g + b.rect[2 + ax]);
            let rec = self.sem(K_SLIDER, pid(id, P_GGRIP, gkey(a.side, a.tag)), Some(tr("Teiler")), len);
            rec.max = pair;
        }
    }

    // the tab strip of a group (side, tag) or of a floating window (DOCK_FLOAT, slot): the
    // surface, the tabs, the buttons and the menu -> the tabs' rectangles
    #[allow(clippy::too_many_arguments)]
    fn dk_header(&mut self, rt: &mut Rt, st: &mut DockState, side: u8, tag: u32, rect: [i32; 4], hdr: [i32; 4], panels: &[u32], active: u32) -> Vec<[i32; 4]> {
        let id = rt.id;
        let key = gkey(side, tag);
        let float = side == DOCK_FLOAT;
        let sh = self.style.shell();
        let focus_in = rt.focus_in.contains(&key);
        let mut sp = Sp { value: focus_in as i32, ..Sp::default() };
        sp.place(rect);
        sh.dock_group(self, &sp);
        sp.place(hdr);
        sh.dock_header(self, &sp);
        // the buttons at the end (a narrow group keeps only the menu), the tabs before them
        let bw = self.dk_metric(SM_DOCK_BTN, BTN).min(hdr[3]);
        let pad = self.dp(4);
        let nb = if hdr[2] - pad * 2 - bw * 3 >= self.dp(72) { 3 } else { 1 };
        let tx = hdr[0] + pad;
        let tw = (hdr[2] - pad * 2 - bw * nb).max(0);
        let mut ws: Vec<i32> = Vec::with_capacity(panels.len());
        for &p in panels {
            let (title, icon) = rt.info(p).map_or((String::new(), 0), |i| (i.title.clone(), i.icon));
            let mut tsp = Sp { label: Some(&title), icon, ..Sp::default() };
            ws.push(sh.dock_tab_size(self, &mut tsp).0);
        }
        let total: i32 = ws.iter().sum();
        if total > tw && total > 0 {
            let min = self.dp(24);
            for w in ws.iter_mut() {
                *w = ((*w as i64 * tw as i64 / total as i64) as i32).max(min);
            }
        }
        let mut tabs = Vec::with_capacity(panels.len());
        let mut x = tx;
        for &w in &ws {
            tabs.push([x, hdr[1], w, hdr[3]]);
            x += w;
        }
        let size = if float {
            st.floats.iter().find(|f| f.slot as u32 == tag).map_or([280, 320], |f| [f.rect[2], f.rect[3]])
        } else {
            [self.px2dip(rect[2]).clamp(FLOAT_W.0, FLOAT_W.1), self.px2dip(rect[3]).clamp(FLOAT_H.0, FLOAT_H.1)]
        };
        let menu = pid(id, P_MENU, key);
        let strip = pid(id, P_STRIP, key);
        // the strip: one focus stop; its empty part drags the whole group
        let sev = self.hit_rect(strip, tx, hdr[1], tw, hdr[3], HB_FOCUS);
        if !self.dk_drag(rt, sev, strip, (side, tag), panels, active, hdr, size) && sev & EV_DOUBLE != 0 {
            rt.acts.push(if float { Act::Home(tag as u8) } else { Act::Float(active, self.dk_float_near(rect, 24, 24)) });
        }
        if st.focus_req != 0 && st.focus_req == active {
            st.focus_req = 0;
            self.focus(strip);
        }
        let n = panels.len();
        let at = panels.iter().position(|&p| p == active).unwrap_or(0);
        if sev & EV_FOCUS != 0 && n > 0 {
            let mut k = at;
            if self.key(VK_LEFT) {
                k = k.saturating_sub(1);
            }
            if self.key(VK_RIGHT) {
                k = (k + 1).min(n - 1);
            }
            if self.key(VK_HOME) {
                k = 0;
            }
            if self.key(VK_END) {
                k = n - 1;
            }
            if k != at {
                rt.acts.push(Act::Activate(panels[k]));
                self.wp.ctx.focus_visible = true;
            }
            if self.key(VK_APPS) || self.dk_key(VK_F10, MOD_SHIFT) {
                self.popup_open(menu);
                self.pp_flag(menu, super::popup::PPF_FOCUS);
                self.wp.ctx.focus_visible = true;
                rt.menu_point = 0;
            }
        }
        // the tabs
        let strip_focus = sev & EV_FOCUS != 0;
        for (i, &p) in panels.iter().enumerate() {
            let r = tabs[i];
            let tid = pid(id, P_TAB, p);
            let ev = self.hit_rect(tid, r[0], r[1], r[2], r[3], 0);
            let state = self.wp.ctx.last_state;
            if !self.dk_drag(rt, ev, tid, (side, tag), &[p], p, hdr, size) {
                if ev & EV_DOUBLE != 0 && rt.flags(p) & DP_NOFLOAT == 0 {
                    rt.acts.push(if float { Act::Home(tag as u8) } else { Act::Float(p, self.dk_float_near(rect, 24, 24)) });
                } else if ev & EV_ACT != 0 {
                    rt.acts.push(Act::Activate(p));
                    self.wp.ctx.focus = strip;
                    self.wp.ctx.focus_visible = false;
                }
                if ev & EV_RCLICK != 0 {
                    rt.acts.push(Act::Activate(p));
                    self.popup_open(menu);
                    rt.menu_point = menu;
                }
            }
            let (title, icon) = rt.info(p).map_or((String::new(), 0), |i| (i.title.clone(), i.icon));
            let mut tsp = Sp { ev, label: Some(&title), icon, value: (p == active) as i32, value2: focus_in as i32, ..Sp::default() };
            tsp.place(r);
            sh.dock_tab(self, &tsp);
            let mut s = state;
            if p == active {
                s |= SS_SELECTED as u32;
                if strip_focus {
                    s |= SS_FOCUSED as u32;
                    if self.wp.ctx.focus_visible {
                        let stl = self.style;
                        stl.focus_ring(self, r, 4);
                    }
                }
            }
            self.wp.ctx.last_rect = r;
            self.wp.ctx.last_state = s;
            self.sem(K_TAB, tid, Some(&title), i as i32);
        }
        // the buttons: menu, collapse (a floating window: dock), hide
        let title = rt.title(active);
        let flags = rt.flags(active);
        let by = hdr[1] + ((hdr[3] - bw) >> 1);
        let bx = hdr[0] + hdr[2] - pad - bw * nb;
        let name = |s: &str| alloc::format!("{}: {}", s, title);
        self.next_rect_px(bx, by, bw, bw);
        self.next_icon(IC_MORE);
        self.next_flags(ST_SUBTLE | ST_ICON_ONLY | ST_COMPACT);
        if self.button(menu, &name(tr("Weitere Optionen"))) & EV_ACT != 0 {
            self.popup_open(menu);
            if self.wp.ctx.focus_visible {
                self.pp_flag(menu, super::popup::PPF_FOCUS);
            }
            rt.menu_point = 0;
        }
        let fl = if rt.menu_point == menu { FL_POINT } else { FL_ALIGN_END };
        if self.flyout_begin(menu, fl) {
            let row = |ui: &mut Ui, k: u32, label: &str, on: bool| -> bool {
                if !on {
                    return false;
                }
                ui.menu_item(pid(id, P_MITEM + k, key), label) & EV_ACT != 0
            };
            if float {
                if row(self, 0, tr("Andocken"), true) {
                    rt.acts.push(Act::Home(tag as u8));
                }
            } else if row(self, 0, tr("Lösen"), flags & DP_NOFLOAT == 0) {
                rt.acts.push(Act::Float(active, self.dk_float_near(rect, 24, 24)));
            }
            for s in 0..3u8 {
                let l = self.dk_side_label(s);
                if row(self, 1 + s as u32, l, s != side) {
                    rt.acts.push(Act::ToSide(active, s));
                }
            }
            self.menu_sep();
            if row(self, 4, tr("Einklappen"), !float) {
                rt.acts.push(Act::Collapse(side, true));
            }
            if row(self, 5, tr("Ausblenden"), flags & DP_NOCLOSE == 0) {
                rt.acts.push(Act::Hide(active));
            }
        } else if rt.menu_point == menu {
            rt.menu_point = 0;
        }
        self.flyout_end();
        if nb == 1 {
            return tabs;
        }
        self.next_rect_px(bx + bw, by, bw, bw);
        if float {
            self.next_icon(IC_PIN);
            self.next_flags(ST_SUBTLE | ST_ICON_ONLY | ST_COMPACT);
            if self.button(pid(id, P_BTN1, key), &name(tr("Andocken"))) & EV_ACT != 0 {
                rt.acts.push(Act::Home(tag as u8));
            }
        } else {
            self.next_icon(match side {
                DOCK_LEFT => IC_CHEV_LEFT,
                DOCK_RIGHT => IC_CHEV_RIGHT,
                _ => IC_CHEV_DOWN,
            });
            self.next_flags(ST_SUBTLE | ST_ICON_ONLY | ST_COMPACT);
            if self.button(pid(id, P_BTN1, key), &name(tr("Einklappen"))) & EV_ACT != 0 {
                rt.acts.push(Act::Collapse(side, true));
            }
        }
        self.next_rect_px(bx + bw * 2, by, bw, bw);
        self.next_icon(IC_CANCEL);
        self.next_flags(ST_SUBTLE | ST_ICON_ONLY | ST_COMPACT | if flags & DP_NOCLOSE != 0 { ST_DISABLED } else { 0 });
        if self.button(pid(id, P_BTN2, key), &name(tr("Ausblenden"))) & EV_ACT != 0 {
            rt.acts.push(Act::Hide(active));
        }
        tabs
    }

    // the drag of a tab or a strip (widget wid, its events ev): starts with EV_DRAG, follows
    // the pointer, drops at the release -> true when the events belonged to the drag
    #[allow(clippy::too_many_arguments)]
    fn dk_drag(&mut self, rt: &mut Rt, ev: u32, wid: u32, src: (u8, u32), panels: &[u32], active: u32, hdr: [i32; 4], size: [i32; 2]) -> bool {
        let win = self.uiw_cur;
        let frame = self.frame;
        if let Some(d) = rt.drag.as_mut() {
            if d.wid != wid || d.win != win {
                return false;
            }
            let (mx, my) = self.mouse();
            let at = self.dk_main_point(mx, my);
            d.seen = frame;
            if ev & EV_HELD != 0 {
                if self.key(VK_ESCAPE) {
                    rt.drag = None;
                    self.wp.ctx.active = 0; // the release is no click
                    self.repass = true;
                } else if d.at != at {
                    d.at = at;
                    self.repass = true;
                }
                return true;
            }
            // released: dropped where the pointer is (dock_end finds the place)
            if let Some(mut d) = rt.drag.take() {
                d.at = at;
                rt.acts.push(Act::Drop(d));
            }
            return true;
        }
        if ev & EV_DRAG != 0 && ev & EV_HELD != 0 && !panels.is_empty() && !self.dragging() {
            let (mx, my) = self.mouse();
            let at = self.dk_main_point(mx, my);
            let (px, py) = (self.wp.ctx.press_x, self.wp.ctx.press_y);
            let grab = [self.px2dip(px - hdr[0]), self.px2dip(py - hdr[1])];
            rt.drag = Some(Drag { panels: panels.to_vec(), active, src, win, wid, grab, size, at, seen: frame });
            self.wp.ctx.last_click_id = 0; // a drag is no first click of a double click
            self.repass = true;
            return true;
        }
        false
    }

    // a floating window: placement, window_begin, its tab strip, the content box -> the panel
    fn dk_float_open(&mut self, rt: &mut Rt, st: &mut DockState, f: usize) -> Option<u32> {
        let fl = st.floats[f].clone();
        let (panels, active) = rt.shown(&fl.group);
        if panels.is_empty() {
            return None;
        }
        let k = fl.slot as usize;
        while rt.wins.len() <= k {
            rt.wins.push(WinSlot::default());
        }
        {
            // the layout's placement, or what the user did to the window
            let w = &mut rt.wins[k];
            let cur = w.rect.get();
            let c = [cur.x, cur.y, cur.w, cur.h];
            if w.applied != fl.rect {
                let r = fl.rect;
                w.rect.set(WinRect::new(r[0], r[1], r[2], r[3], WRF_POS));
                w.applied = fl.rect;
            } else if c != w.applied && cur.flags & WRF_POS != 0 {
                st.floats[f].rect = c;
                w.applied = c;
                rt.changed = true;
            }
            w.open.set(1);
        }
        let wid = rt.id.wrapping_add(1 + k as u32);
        let title = rt.title(active);
        let (open, rect) = (rt.wins[k].open.clone(), rt.wins[k].rect.clone());
        if !self.window_begin(wid, &title, Some(&open), WF_TOOL | WF_RESIZABLE, Some(&rect)) {
            self.window_end();
            return None;
        }
        rt.wins[k].used = true;
        let r = self.box_rect();
        let hh = self.dk_metric(SM_DOCK_HDR, HDR).min(r[3]);
        let hdr = [r[0], r[1], r[2], hh];
        self.dk_header(rt, st, DOCK_FLOAT, k as u32, r, hdr, &panels, active);
        self.next_rect_px(r[0], r[1] + hh, r[2], r[3] - hh);
        self.box_begin(pid(rt.id, P_CONTENT, active), BOX_V | BOX_CLIP | BOX_FIXED, 0);
        rt.open = 2;
        rt.open_key = gkey(DOCK_FLOAT, k as u32);
        rt.open_panels = panels;
        rt.open_active = active;
        rt.hits0 = 0;
        Some(active)
    }

    // close what dock_next opened: Ctrl+PgDn / Ctrl+PgUp while the focus is in the group
    fn dk_close(&mut self, rt: &mut Rt) {
        let open = core::mem::take(&mut rt.open);
        if open == 0 {
            return;
        }
        if open == 1 {
            self.focus_pane_end();
        }
        self.box_end();
        let key = rt.open_key;
        if key != u32::MAX {
            let strip = pid(rt.id, P_STRIP, key);
            let focus = self.wp.ctx.focus;
            let c = &self.wp.ctx;
            let hits = &c.hits[c.hit_cur];
            let from = rt.hits0.min(hits.len());
            let here = focus != 0 && (focus == strip || hits[from..].iter().any(|h| h.id == focus));
            if here {
                rt.focus_now.push(key);
                let n = rt.open_panels.len();
                let at = rt.open_panels.iter().position(|&p| p == rt.open_active).unwrap_or(0);
                let mut k = at;
                if n > 1 && self.dk_key(VK_NEXT, MOD_CTRL) {
                    k = (at + 1) % n;
                }
                if n > 1 && self.dk_key(VK_PRIOR, MOD_CTRL) {
                    k = (at + n - 1) % n;
                }
                if k != at {
                    rt.acts.push(Act::Activate(rt.open_panels[k]));
                    self.focus(strip);
                }
            }
        }
        if open == 2 {
            self.window_end();
        }
    }

    // ------------------------------------------------------------------ drop markers

    // the markers of a drag: on the group under the pointer, at the area's edges
    fn dk_marks(&self, rt: &Rt, d: &Drag) -> Vec<Mark> {
        let mut v = Vec::new();
        let a = rt.area;
        let at = d.at;
        if !inside(a, at) {
            return v;
        }
        let m = self.dp(MARK);
        let gap = self.dp(4);
        if let Some(g) = rt.groups.iter().find(|g| inside(g.rect, at)) {
            if !self.dk_whole(d, g) {
                let (cx, cy) = (g.body[0] + g.body[2] / 2, g.body[1] + g.body[3] / 2);
                let c = [cx - m / 2, cy - m / 2, m, m];
                v.push(Mark { kind: DOCK_MK_CENTER, rect: c, to: Target::Tab(g.side, g.tag, usize::MAX), label: "Als Register" });
                if g.side == DOCK_BOTTOM {
                    v.push(Mark { kind: DOCK_MK_LEFT, rect: [c[0] - m - gap, c[1], m, m], to: Target::Split(g.side, g.tag, false), label: "Davor" });
                    v.push(Mark { kind: DOCK_MK_RIGHT, rect: [c[0] + m + gap, c[1], m, m], to: Target::Split(g.side, g.tag, true), label: "Dahinter" });
                } else {
                    v.push(Mark { kind: DOCK_MK_TOP, rect: [c[0], c[1] - m - gap, m, m], to: Target::Split(g.side, g.tag, false), label: "Darüber" });
                    v.push(Mark { kind: DOCK_MK_BOTTOM, rect: [c[0], c[1] + m + gap, m, m], to: Target::Split(g.side, g.tag, true), label: "Darunter" });
                }
            }
        }
        let e = self.dp(12);
        let my = a[1] + (a[3] - m) / 2;
        v.push(Mark { kind: DOCK_MK_LEFT, rect: [a[0] + e, my, m, m], to: Target::Side(DOCK_LEFT), label: "Links andocken" });
        v.push(Mark { kind: DOCK_MK_RIGHT, rect: [a[0] + a[2] - e - m, my, m, m], to: Target::Side(DOCK_RIGHT), label: "Rechts andocken" });
        let c = rt.center;
        v.push(Mark { kind: DOCK_MK_BOTTOM, rect: [c[0] + (c[2] - m) / 2, a[1] + a[3] - e - m, m, m], to: Target::Side(DOCK_BOTTOM), label: "Unten andocken" });
        v
    }

    // the drag carries every panel of group g (it would land where it is)
    fn dk_whole(&self, d: &Drag, g: &Geo) -> bool {
        d.src == (g.side, g.tag) && g.panels.iter().all(|p| d.panels.contains(p))
    }

    // where a release at d.at puts the dragged panels
    fn dk_target(&self, rt: &Rt, d: &Drag) -> Target {
        let at = d.at;
        if let Some(mk) = self.dk_marks(rt, d).into_iter().find(|mk| inside(mk.rect, at)) {
            return mk.to;
        }
        for g in &rt.groups {
            if inside(g.hdr, at) {
                if self.dk_whole(d, g) {
                    return Target::None;
                }
                let i = g.tabs.iter().filter(|t| t[0] + t[2] / 2 < at[0]).count();
                return Target::Tab(g.side, g.tag, i);
            }
        }
        if d.panels.iter().any(|&p| rt.flags(p) & DP_NOFLOAT != 0) {
            return Target::None;
        }
        Target::Float
    }

    // the place a target shows while the pointer is over it (main window px)
    fn dk_preview(&self, rt: &Rt, st: &DockState, d: &Drag, to: Target) -> Option<[i32; 4]> {
        let half = |r: [i32; 4], side: u8, end: bool| {
            let ax = if side == DOCK_BOTTOM { 0 } else { 1 };
            let mut q = r;
            q[2 + ax] = r[2 + ax] / 2;
            if end {
                q[ax] = r[ax] + r[2 + ax] - q[2 + ax];
            }
            q
        };
        let group = |s: u8, t: u32| rt.groups.iter().find(|g| g.side == s && g.tag == t);
        match to {
            Target::None => None,
            Target::Tab(s, t, _) => group(s, t).map(|g| g.rect),
            Target::Split(s, t, after) => group(s, t).map(|g| half(g.rect, s, after)),
            Target::Side(s) => {
                if rt.on[s as usize] == 1 {
                    return rt.groups.iter().rev().find(|g| g.side == s).map(|g| half(g.rect, s, true));
                }
                let a = rt.area;
                let c = rt.center;
                let n = self.dp(st.size(s));
                Some(match s {
                    DOCK_LEFT => [a[0], a[1], n.min(a[2] / 2), a[3]],
                    DOCK_RIGHT => [a[0] + a[2] - n.min(a[2] / 2), a[1], n.min(a[2] / 2), a[3]],
                    _ => [c[0], a[1] + a[3] - n.min(a[3] / 2), c[2], n.min(a[3] / 2)],
                })
            }
            Target::Float => {
                let fb = &self.wp.rt.fb;
                if !inside([0, 0, fb.w, fb.h], d.at) {
                    return None;
                }
                Some([d.at[0] - self.dp(d.grab[0]), d.at[1] - self.dp(d.grab[1]), self.dp(d.size[0]), self.dp(d.size[1])])
            }
        }
    }

    // (dock_end, the main window) the drag's preview and markers over everything
    fn dk_overlay(&mut self, rt: &Rt, st: &DockState) {
        let Some(d) = rt.drag.clone() else { return };
        let to = self.dk_target(rt, &d);
        let marks = self.dk_marks(rt, &d);
        let prev = self.layer_set(L_POPUP);
        let (lr, ls) = (self.wp.ctx.last_rect, self.wp.ctx.last_state);
        let sh = self.style.shell();
        if let Some(r) = self.dk_preview(rt, st, &d, to) {
            let mut sp = Sp { value: (to == Target::Float) as i32, ..Sp::default() };
            sp.place(r);
            sh.dock_preview(self, &sp);
        }
        for (k, mk) in marks.iter().enumerate() {
            let hover = inside(mk.rect, d.at);
            let mut sp = Sp { value: mk.kind, ev: if hover { EV_HOVER } else { 0 }, ..Sp::default() };
            sp.place(mk.rect);
            sh.dock_marker(self, &sp);
            let label = match mk.to {
                Target::Side(s) => self.dk_side_label(s),
                _ => tr(mk.label),
            };
            self.wp.ctx.last_rect = mk.rect;
            self.wp.ctx.last_state = if hover { SS_HOVER as u32 } else { 0 };
            self.sem(K_BUTTON, pid(rt.id, P_MARK, k as u32), Some(label), 0);
        }
        self.wp.ctx.last_rect = lr;
        self.wp.ctx.last_state = ls;
        self.layer_set(prev);
    }

    // ------------------------------------------------------------------ changes

    fn dk_apply(&mut self, rt: &mut Rt, st: &mut DockState, a: Act) {
        match a {
            Act::Activate(p) => st.activate(p),
            Act::Hide(p) => st.hide(p),
            Act::Collapse(s, on) => st.collapse(s, on),
            Act::Home(slot) => st.dock_home(slot),
            Act::ToSide(p, s) => st.dock_to(p, s),
            Act::Float(p, r) => {
                st.float(p, Some(r));
            }
            Act::Drop(d) => {
                let to = self.dk_target(rt, &d);
                self.dk_drop(st, d, to);
            }
        }
        rt.changed = true;
        self.repass = true;
    }

    // the dragged panels to their new place
    fn dk_drop(&mut self, st: &mut DockState, d: Drag, to: Target) {
        let panels: Vec<u32> = d.panels.iter().copied().filter(|&p| st.visible(p)).collect();
        if panels.is_empty() {
            return;
        }
        let active = if panels.contains(&d.active) { d.active } else { panels[0] };
        match to {
            Target::None => {}
            Target::Side(s) => {
                for &p in &panels {
                    st.take(p);
                }
                st.put_group(s as usize, None, &panels, active);
            }
            Target::Tab(s, tag, i) => {
                // the index among the tabs that stay
                let mut i = i;
                if let Some(g) = st.sides[s as usize].groups.iter().find(|g| g.tag == tag) {
                    let end = i.min(g.panels.len());
                    i = end - g.panels[..end].iter().filter(|p| panels.contains(p)).count();
                }
                for &p in &panels {
                    st.take(p);
                }
                st.put_tabs(s as usize, tag, i, &panels, active);
            }
            Target::Split(s, tag, after) => {
                for &p in &panels {
                    st.take(p);
                }
                let there = st.sides[s as usize].groups.iter().any(|g| g.tag == tag);
                st.put_group(s as usize, if there { Some((tag, after)) } else { None }, &panels, active);
            }
            Target::Float => {
                // the window's top left: the strip under the pointer as it was grabbed
                let px = if self.rtl { self.wp.rt.fb.w - 1 - d.at[0] } else { d.at[0] };
                let (x, y) = (self.px2dip(px), self.px2dip(d.at[1]));
                let (w, h) = (d.size[0], d.size[1]);
                let gx = if self.rtl { w - d.grab[0] } else { d.grab[0] };
                let tbh = self.style.metric(SM_WIN_TBH_TOOL);
                let r = [x - gx, y - d.grab[1] - tbh, w, h];
                // a whole floating window moves
                if d.src.0 == DOCK_FLOAT {
                    if let Some(f) = st.floats.iter_mut().find(|f| f.slot as u32 == d.src.1) {
                        if f.group.panels.iter().all(|p| panels.contains(p)) {
                            f.rect[0] = r[0];
                            f.rect[1] = r[1];
                            return;
                        }
                    }
                }
                let homes: Vec<Hidden> = panels.iter().filter_map(|&p| st.take(p)).collect();
                let (home, home_tag) = match homes.first() {
                    Some(h) if h.side < DOCK_FLOAT => (h.side, h.tag),
                    Some(h) => (h.home, h.home_tag),
                    None => (DOCK_RIGHT, 0),
                };
                if !st.put_float(r, home, home_tag, &panels, active) {
                    for h in homes {
                        st.restore(h);
                    }
                }
            }
        }
    }
}
