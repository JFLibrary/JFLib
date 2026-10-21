// rtl -- right-to-left layout: every window mirrored as a whole (as Windows' WS_EX_LAYOUTRTL)
//
//   rtl() -> bool                    the windows are mirrored now
//   set_rtl(Some(on) | None)         force it, or None: from the UI language (Arabic, Hebrew,
//                                    Persian, Urdu, ...: i18n::rtl_language); command line
//                                    --rtl / --ltr
//   ltr_begin(x, w) / ltr_end()      a region of the window whose content keeps its reading
//                                    order (text fields, canvases, colour pads): it moves to its
//                                    mirrored place as a whole
//   ltr_x(x) -> x                    a logical x (the pointer: wp.ctx.mx) inside the outermost
//                                    open region, so the region's own maths stays LTR
//
// Layout and hit testing stay in logical (LTR) coordinates: a box's first child is its
// logical start.  Before a window is rendered, every recorded command is mirrored across
// it (cmd_mirror): boxes, clips, lines and shapes flip; text, images and custom drawings
// move but keep their direction; directional icons (chevrons, back / forward, undo / redo,
// indent) are swapped for their mirror images.  Input comes back the same way: the pointer x
// is mirrored, a horizontal wheel turns round, plain Left / Right swap (text fields swap
// them back).  What leaves the UI is physical: sems (scripts, the agent), popup rects of OS
// popup windows, the platform's hit tests, resize edges and the IME caret.

use alloc::vec::Vec;

use super::consts::*;
use super::keys::*;
use super::Ui;
use crate::cmd::Cmd;

/// a region that keeps its LTR content: records [start, end) of a layer
#[derive(Clone, Copy)]
pub(crate) struct Island {
    layer: u8,
    start: u32,
    end: u32,
    x: i32,
    w: i32,
}

/// the per-window part (with the command lists)
#[derive(Default)]
pub(crate) struct W {
    pub(crate) islands: Vec<Island>,
    /// open ltr_begin regions (indices into islands; usize::MAX: not recorded)
    open: Vec<usize>,
    /// the command lists are mirrored already (cleared with them)
    pub(crate) mirrored: bool,
}

impl W {
    pub(crate) fn reset(&mut self) {
        self.islands.clear();
        self.open.clear();
        self.mirrored = false;
    }
}

// icons that point somewhere: each one is drawn as its partner in a mirrored window
const MIRROR_ICONS: [(u32, u32); 11] = [
    (IC_CHEV_LEFT, IC_CHEV_RIGHT),
    (IC_CHEV_LEFT_SMALL, IC_CHEV_RIGHT_SMALL),
    (IC_CARET_LEFT, IC_CARET_RIGHT),
    (IC_BACK, IC_FORWARD),
    (IC_PREVIOUS, IC_NEXT),
    (IC_UNDO, IC_REDO),
    (IC_OFFICE_CHEVRON_LEFT, IC_OFFICE_CHEVRON_RIGHT),
    (IC_OFFICE_PREVIOUS, IC_OFFICE_NEXT),
    (IC_OFFICE_UNDO, IC_OFFICE_REDO),
    (IC_OFFICE_INDENT, IC_OFFICE_OUTDENT),
    (IC_OFFICE_INSERT_LEFT, IC_OFFICE_INSERT_RIGHT),
];

/// the mirror image of an icon (itself when it has no direction)
pub fn mirror_icon(cp: u32) -> u32 {
    for (a, b) in MIRROR_ICONS {
        if cp == a {
            return b;
        }
        if cp == b {
            return a;
        }
    }
    cp
}

impl Ui {
    /// the windows are mirrored now (right-to-left layout)
    pub fn rtl(&self) -> bool {
        self.rtl
    }

    /// mirror the windows (Some(true)), never (Some(false)) or as the UI language reads (None)
    pub fn set_rtl(&mut self, on: Option<bool>) {
        self.rtl_mode = match on {
            None => 0,
            Some(true) => 1,
            Some(false) => 2,
        };
        self.rtl_update();
    }

    // frame_begin: the mode for this frame
    pub(crate) fn rtl_update(&mut self) {
        let on = match self.rtl_mode {
            1 => true,
            2 => false,
            _ => crate::i18n::rtl_language(),
        };
        if on != self.rtl {
            self.rtl = on;
            self.dirty_force_full();
        }
    }

    /// ltr_begin(x, w): the following commands of this layer keep their LTR order inside the
    /// columns x .. x + w (px); pairs with ltr_end
    pub fn ltr_begin(&mut self, x: i32, w: i32) {
        let layer = self.wp.cmd.layer;
        let start = self.wp.cmd.layers[layer].len() as u32;
        let r = &mut self.wp.rtl;
        if !self.rtl {
            r.open.push(usize::MAX);
            return;
        }
        r.islands.push(Island { layer: layer as u8, start, end: u32::MAX, x, w });
        r.open.push(r.islands.len() - 1);
    }

    /// close the region of ltr_begin
    pub fn ltr_end(&mut self) {
        let Ui { wp, .. } = self;
        if let Some(k) = wp.rtl.open.pop() {
            if let Some(i) = wp.rtl.islands.get_mut(k) {
                // the end in the region's own layer (a popup opened inside does not count)
                i.end = wp.cmd.layers[i.layer as usize].len() as u32;
            }
        }
    }

    // the outermost open region (x, w)
    fn ltr_outer(&self) -> Option<(i32, i32)> {
        let r = &self.wp.rtl;
        r.open.iter().find(|&&k| k != usize::MAX).and_then(|&k| r.islands.get(k)).map(|i| (i.x, i.w))
    }

    /// a logical x (the pointer) as the open ltr_begin region sees it: its content is not
    /// mirrored, so the pointer is mirrored back across the region
    pub fn ltr_x(&self, x: i32) -> i32 {
        match self.ltr_outer() {
            Some((ix, iw)) => self.rtl_map(x, ix, iw),
            None => x,
        }
    }

    /// rtl_map(x, ix, iw): a logical x as a region ix .. ix + iw that keeps its LTR content
    /// sees it (the same before its ltr_begin)
    pub fn rtl_map(&self, x: i32, ix: i32, iw: i32) -> i32 {
        if self.rtl { ix.wrapping_add(ix).wrapping_add(iw).wrapping_sub(1).wrapping_sub(x) } else { x }
    }

    /// a logical point x of this window in window px (the IME caret): mirrored, or moved with
    /// the open ltr_begin region
    pub(crate) fn phys_x(&self, x: i32) -> i32 {
        if !self.rtl {
            return x;
        }
        let w = self.wp.rt.fb.w;
        match self.ltr_outer() {
            Some((ix, iw)) => x.wrapping_add(w - ix - ix - iw),
            None => w.wrapping_sub(x),
        }
    }

    /// a logical rectangle of this window in window px
    pub(crate) fn phys_rect(&self, r: [i32; 4]) -> [i32; 4] {
        if !self.rtl {
            return r;
        }
        [self.wp.rt.fb.w.wrapping_sub(r[0]).wrapping_sub(r[2]), r[1], r[2], r[3]]
    }

    // input_event: the pointer and the keys of a mirrored window into logical ones
    pub(crate) fn rtl_input(&self, ev: &mut plat::Event) {
        if !self.rtl {
            return;
        }
        match ev.typ {
            plat::E_MOVE | plat::E_DOWN | plat::E_UP if ev.x != -1 => {
                ev.x = self.wp.rt.fb.w.wrapping_sub(1).wrapping_sub(ev.x);
            }
            plat::E_WHEEL => {
                // the horizontal part (high 16 bits) turns round
                let wx = (ev.data as i32) >> 16;
                ev.data = (ev.data & 0xFFFF) | ((wx.wrapping_neg() as u32) << 16);
            }
            plat::E_KEY_DOWN if ev.mods as u32 & (MOD_CTRL | MOD_ALT) == 0 => {
                ev.data = swap_arrow(ev.data);
            }
            _ => {}
        }
    }

    /// a key of the queue as a text field reads it: Left / Right swapped back in a mirrored
    /// window (text keeps its reading order)
    pub(crate) fn rtl_text_key(&self, k: u32) -> u32 {
        if self.rtl && (k >> 16) & (MOD_CTRL | MOD_ALT) == 0 { swap_arrow(k) } else { k }
    }

    /// cmd_mirror: mirror the recorded commands of the live window across it (once per
    /// recording; render_live)
    pub(crate) fn cmd_mirror(&mut self) {
        if !self.rtl || self.wp.rtl.mirrored {
            return;
        }
        self.wp.rtl.mirrored = true;
        let w = self.wp.rt.fb.w;
        let Ui { wp, text, .. } = self;
        for (l, list) in wp.cmd.layers.iter_mut().enumerate() {
            // the move of each record: None = mirror, Some(d) = shift by d (in an island)
            let mut shift: Vec<Option<i32>> = alloc::vec![None; list.len()];
            for i in wp.rtl.islands.iter().filter(|i| i.layer as usize == l) {
                let d = w - i.x - i.x - i.w;
                let end = (i.end as usize).min(list.len());
                for s in shift.iter_mut().take(end).skip(i.start as usize) {
                    if s.is_none() {
                        *s = Some(d); // outer regions come first: they decide
                    }
                }
            }
            for (c, s) in list.iter_mut().zip(shift) {
                match s {
                    Some(d) => shift_cmd(c, d),
                    None => mirror_cmd(c, w, text),
                }
            }
        }
    }
}

fn swap_arrow(k: u32) -> u32 {
    match k & 0xFFFF {
        VK_LEFT => (k & !0xFFFF) | VK_RIGHT,
        VK_RIGHT => (k & !0xFFFF) | VK_LEFT,
        _ => k,
    }
}

// a record moved by d px
fn shift_cmd(c: &mut Cmd, d: i32) {
    match c {
        Cmd::Unclip => {}
        Cmd::Circle { x, .. } | Cmd::PanelFx { x, .. } => *x = x.wrapping_add(d << 8),
        Cmd::Line { x0, x1, .. } => {
            *x0 = x0.wrapping_add(d << 8);
            *x1 = x1.wrapping_add(d << 8);
        }
        Cmd::Rect { x, .. }
        | Cmd::RRect { x, .. }
        | Cmd::Text { x, .. }
        | Cmd::Icon { x, .. }
        | Cmd::Image { x, .. }
        | Cmd::Clip { x, .. }
        | Cmd::Custom { x, .. }
        | Cmd::Shadow { x, .. } => *x = x.wrapping_add(d),
    }
}

// a record mirrored across a window w px wide: boxes flip, text / images / custom drawings
// keep their direction at the mirrored place, directional icons swap
fn mirror_cmd(c: &mut Cmd, w: i32, text: &mut text::Text) {
    let w8 = w << 8;
    match c {
        Cmd::Unclip => {}
        Cmd::Circle { x, .. } => *x = w8.wrapping_sub(*x),
        Cmd::PanelFx { x, w: pw, .. } => *x = w8.wrapping_sub(*x).wrapping_sub(*pw),
        Cmd::Line { x0, x1, .. } => {
            *x0 = w8.wrapping_sub(*x0);
            *x1 = w8.wrapping_sub(*x1);
        }
        Cmd::Rect { x, w: rw, .. }
        | Cmd::RRect { x, w: rw, .. }
        | Cmd::Image { x, w: rw, .. }
        | Cmd::Clip { x, w: rw, .. }
        | Cmd::Custom { x, w: rw, .. }
        | Cmd::Shadow { x, w: rw, .. } => *x = w.wrapping_sub(*x).wrapping_sub(*rw),
        Cmd::Text { x, font, size, maxw, s, .. } => {
            text.set_font(*font as u32, *size);
            let mut tw = text.width_n(s, s.len());
            if *maxw != 0 {
                tw = tw.min(*maxw);
            }
            *x = w.wrapping_sub(*x).wrapping_sub(tw);
        }
        Cmd::Icon { x, cp, size, .. } => {
            *x = w.wrapping_sub(*x).wrapping_sub(((*size as u32) >> 8) as i32);
            *cp = mirror_icon(*cp);
        }
    }
}
