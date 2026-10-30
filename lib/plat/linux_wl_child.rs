// linux_wl_child -- child windows on Wayland: tool windows as subsurfaces of their owner
//
// Wayland clients cannot place toplevels: every compositor puts an owned toplevel where it
// likes (GNOME, KDE and WSLg centre it on its parent), so tool windows that the app docks at
// the edges of its main window all appeared in the middle.  A tool window (PWF_TOOL without
// PWF_DIALOG; jfgui_WL_TOOLWIN=toplevel turns this off) is therefore a child window
// (WLWF_CHILD): a desynchronized wl_subsurface of its owner's surface, with its own
// wl_surface, viewport and fractional scale, but no xdg role.
//
//   position     cx, cy: px of the owner, relative to its surface origin; sent as logical
//                set_position (owner scale), applied with the owner's next commit
//                (commit_parent, or the owner's next present).  win_get_rect answers it,
//                win_state carries WST_CHILD, E_WINPOS reports it (relative)
//   inside       the window stays inside its owner (clamp); when the owner shrinks, its
//                children are moved back in (E_WINPOS); when its scale changes they keep
//                their place in its content
//   pixel grid   positions and sizes lie on whole logical units that are whole physical
//                pixels (grid: 1.25 -> every 4th logical unit = 5 px, 1.5 -> every 2nd = 3 px,
//                2 -> every unit = 2 px): compositors do not round subsurface positions (half a
//                pixel off blurs the whole window), the viewport maps a size on the grid 1:1,
//                an integer buffer scale needs sizes that are multiples of it
//   stacking     new children go on top; win_activate raises one (place_above)
//   keyboard     the keyboard focus stays with the root toplevel; key events go to the child
//                in kb_child while it is activated: a press on a text box (the UI's hit hook,
//                HB_TEXT, as WM_MOUSEACTIVATE on Windows) or win_activate.  A press on the root
//                or activating the root gives the keys back (E_FOCUS_OUT / E_FOCUS_IN as on
//                Windows)
//   owner gone   win_destroy of the owner destroys its children's objects; their records
//                stay (as Windows keeps the slot of a destroyed owned window) but are never
//                shown again, until the UI destroys them
//   move / size  win_begin_move / win_begin_resize start a drag of our own while the mouse
//                button (or the pen tip) that pressed on the child is held: the pointer keeps
//                its implicit grab on the child.  A new position goes out with a commit of the
//                owner and a wl_display.sync; one is in flight at a time.  The positions the
//                compositor reports during the grab are relative to either
//                  - the child's current place (mutter, KWin, Weston): events before the
//                    sync's answer to the old position, later ones to the new one (the
//                    compositor applies the commit and answers the sync in one go), or
//                  - the child's place at the press (sway / wlroots: the implicit grab keeps
//                    its reference point).
//                Which one (model) is found out once per run with the relative-pointer
//                protocol (zwp_relative_pointer_v1) during the first mouse drags: after a sync
//                answer moved the child by J, the next motion changes the position by the
//                relative motion minus J (current) or by the relative motion alone (press).
//                Until then, and without that protocol, "current" holds.  The move hook snaps
//                (the UI's window_snap); E_WINPOS / E_RESIZE follow; the release of that
//                button (the pen's up) ends it.  Touch is not handled (no wl_touch)

use super::proto::*;
use super::wire::{Args, A};
use super::window::{slot, WLWF_CHILD, WLWF_DEAD, WLWF_USED};
use super::Wl;
use crate::{EDGE_BOTTOM, EDGE_LEFT, EDGE_RIGHT, EDGE_TOP, E_FOCUS_IN, E_FOCUS_OUT, E_RESIZE, E_WINPOS, HB_TEXT, PWF_NOACTIVATE,
            PW_KEEP, PW_MAX};

/// the reference of the pointer positions during an implicit grab (Drag::model)
const MODEL_UNKNOWN: u8 = 0;
const MODEL_CURRENT: u8 = 1; // the child's current position
const MODEL_PRESS: u8 = 2; // the child's position at the press

/// a move or resize of a child window by the library
#[derive(Clone, Copy, Default)]
pub(crate) struct Drag {
    pub on: bool,
    pub w: usize,
    pub edges: u32,     // 0 = move, else EDGE_* bits
    pub tool: i32,      // the pen (tablet tool slot) that drags, -1 = the pointer
    pub btns: u32,      // the pointer's held buttons at the start (BTN_* bits): their release ends it
    start: [i32; 4],    // cx, cy, pw, ph at the start (px)
    grab: [i32; 2],     // the pointer in the owner at the start (px)
    applied: [i32; 2],  // the child's position the compositor has (the last sync answered)
    goal: [i32; 2],     // the position wanted (sent when none is in flight)
    sent: [i32; 2],     // the position in flight (cb != 0)
    cb: u32,            // wl_callback of the wl_display.sync after it (0 = none in flight)
    // finding out the model (kept from drag to drag: model)
    model: u8,
    pub other: bool,    // this pointer frame has a button or wheel event
    rp: u32,            // zwp_relative_pointer_v1 while the model is unknown (0 = none)
    cur: Option<[i32; 2]>, // the pointer's last reported position (px of the child)
    lp: Option<[i32; 2]>, // the pointer's position of this pointer frame (px of the child)
    rel: [i32; 2],      // relative motion of this frame (24.8 logical)
    nrel: u32,
    last: [i32; 2],     // the position of the last frame
    last_rel: bool,     // ... which had relative motion too
    jump: [i32; 2],     // moves the compositor applied since then (px)
    vote: u8,
    votes: u8,
}

impl Wl {
    /// the toplevel a (child) window belongs to
    pub(crate) fn root_of(&self, w: usize) -> usize {
        let mut i = w & (PW_MAX - 1);
        for _ in 0..PW_MAX {
            let r = &self.win[i];
            if r.flags & WLWF_CHILD == 0 || r.owner < 0 {
                break;
            }
            i = slot(r.owner);
        }
        i
    }

    /// is w a live child window?
    pub(crate) fn is_child(&self, w: i32) -> bool {
        (0..PW_MAX as i32).contains(&w) && self.win[w as usize].flags & (WLWF_USED | WLWF_CHILD) == WLWF_USED | WLWF_CHILD
    }

    /// offset of window w's surface in its root toplevel's surface (px of the root)
    pub(crate) fn child_offset(&self, w: usize) -> (i32, i32) {
        let (mut x, mut y) = (0, 0);
        let mut i = w & (PW_MAX - 1);
        for _ in 0..PW_MAX {
            let r = &self.win[i];
            if r.flags & WLWF_CHILD == 0 || r.owner < 0 {
                break;
            }
            x += r.cx;
            y += r.cy;
            i = slot(r.owner);
        }
        (x, y)
    }

    /// px of window o -> its logical units
    fn logical_of(&self, o: usize, px: i32) -> i32 {
        let r = &self.win[o & (PW_MAX - 1)];
        let s = r.s120.max(1);
        if r.frac { (px.wrapping_mul(120) + if px >= 0 { s / 2 } else { -(s / 2) }) / s } else { px.wrapping_mul(120) / s }
    }

    /// the pixel grid of window o's scale: (logical units, px) between grid points; None for
    /// a scale without a usable one
    fn grid(&self, o: usize) -> Option<(i32, i32)> {
        let s = self.win[o & (PW_MAX - 1)].s120.max(1);
        let (mut a, mut b) = (s, 120);
        while b != 0 {
            (a, b) = (b, a % b);
        }
        let step = 120 / a;
        if step > 8 { None } else { Some((step, s / a)) }
    }

    /// px of owner o -> the nearest grid position (within lo..=hi when the grid allows)
    fn snap_px(&self, o: usize, px: i32, lo: i32, hi: i32) -> i32 {
        let Some((step, _)) = self.grid(o) else { return px };
        if step == 1 && self.win[o & (PW_MAX - 1)].s120 == 120 {
            return px;
        }
        let s = self.win[o & (PW_MAX - 1)].s120.max(1);
        let to_px = |l: i32| l * s / 120;
        let l = (px * 120).div_euclid(s); // the logical unit at or before px
        let l = l - l.rem_euclid(step);
        // the nearer of the grid points around px
        let mut l = if px - to_px(l) > to_px(l + step) - px { l + step } else { l };
        if to_px(l) > hi && to_px(l - step) >= lo {
            l -= step;
        } else if to_px(l) < lo && to_px(l + step) <= hi {
            l += step;
        }
        to_px(l)
    }

    /// a size (px) of child i on its grid: the nearest multiple, at least one step
    fn snap_size(&self, i: usize, px: i32) -> i32 {
        let Some((_, gp)) = self.grid(i) else { return px };
        (((px + gp / 2) / gp) * gp).max(gp)
    }

    /// (objects_create) the subsurface of a child window: desynchronized, on top of its
    /// siblings, at its position
    pub(crate) fn child_objects_create(&mut self, w: usize) {
        let o = slot(self.win[w].owner);
        let (surf, ps) = (self.win[w].surf, self.win[o].surf);
        let sub = self.w.send(self.g.subc, &SUBCOMPOSITOR_GET_SUBSURFACE, &[A::N(&IFACE_WL_SUBSURFACE), A::O(surf), A::O(ps)]);
        self.win[w].sub = sub;
        self.w.send(sub, &SUBSURFACE_SET_DESYNC, &[]);
        self.g.zc = self.g.zc.wrapping_add(1);
        self.win[w].z = self.g.zc;
        self.child_position(w);
    }

    /// send the child's position (applied with the owner's next commit)
    fn child_position(&mut self, w: usize) {
        let r = self.win[w];
        if r.sub == 0 || r.owner < 0 {
            return;
        }
        let o = slot(r.owner);
        let (lx, ly) = (self.logical_of(o, r.cx), self.logical_of(o, r.cy));
        self.w.send(r.sub, &SUBSURFACE_SET_POSITION, &[A::I(lx), A::I(ly)]);
    }

    /// commit the owner of child w, so its subsurface state (position, stacking) applies; an
    /// owner without a buffer yet applies it with its first present
    fn commit_parent(&mut self, w: usize) {
        if self.win[w].owner < 0 {
            return;
        }
        let o = slot(self.win[w].owner);
        let r = self.win[o];
        if r.surf != 0 && r.mapped {
            self.w.send(r.surf, &SURFACE_COMMIT, &[]);
        }
    }

    /// (win_show) show a child window: its objects (or later, when its owner has no surface)
    pub(crate) fn child_show(&mut self, i: usize) {
        if self.win[i].flags & WLWF_DEAD != 0 || self.win[i].owner < 0 {
            return; // its owner is gone
        }
        let o = slot(self.win[i].owner);
        if self.win[o].surf == 0 {
            self.win[i].hidpar = true; // made when the owner is shown
            return;
        }
        self.win[i].hidpar = false;
        if self.win[i].surf == 0 {
            self.objects_create(i);
        }
        let r = &mut self.win[i];
        r.shown = true;
        r.configured = true; // no configure: its size is ours
        self.update_scale(i as i32);
        self.child_clamp(i, true);
        self.child_position(i);
        self.commit_parent(i);
        self.children_show(i);
        self.w.flush();
    }

    /// window w got its surface: its children that wait for it are shown
    pub(crate) fn children_show(&mut self, w: usize) {
        for c in 0..PW_MAX {
            if c != w && self.is_child(c as i32) && self.win[c].owner == w as i32 && self.win[c].hidpar {
                self.child_show(c);
            }
        }
    }

    /// (win_destroy of w) the windows w owns lose their owner: their objects go, they are
    /// never shown again (their records stay until the UI destroys them)
    pub(crate) fn owned_orphan(&mut self, w: usize) {
        for c in 0..PW_MAX {
            if c != w && self.win[c].flags & WLWF_USED != 0 && self.win[c].owner == w as i32 {
                self.objects_destroy(c);
                let r = &mut self.win[c];
                r.flags |= WLWF_DEAD;
                r.owner = -1;
                r.hidpar = false;
                r.hidmin = false;
            }
        }
    }

    /// (objects_destroy) child w lost its surface: its drag and keyboard end
    pub(crate) fn child_unmapped(&mut self, w: usize) {
        if self.drag.on && self.drag.w == w {
            self.drag_stop();
        }
        if self.inp.kb_child == w as i32 {
            self.inp.kb_child = -1;
        }
    }

    /// (win_set_rect) size as for toplevels, but on the grid (the UI takes the size it got
    /// with win_size, as for every size of its own), x / y relative to the owner (PW_KEEP
    /// keeps)
    pub(crate) fn child_set_rect(&mut self, i: usize, x: i32, y: i32, cw: i32, ch: i32) {
        if cw != PW_KEEP && cw > 0 {
            let w = self.snap_size(i, cw);
            self.win[i].pw = w;
            let l = self.logical_of(i, w);
            self.win[i].lw = l;
            self.win[i].normal_w = l;
        }
        if ch != PW_KEEP && ch > 0 {
            let h = self.snap_size(i, ch);
            self.win[i].ph = h;
            let l = self.logical_of(i, h);
            self.win[i].lh = l;
            self.win[i].normal_h = l;
        }
        if self.win[i].owner < 0 {
            return; // its owner is gone
        }
        let (ox, oy) = (self.win[i].cx, self.win[i].cy);
        let asked = (if x != PW_KEEP { x } else { ox }, if y != PW_KEEP { y } else { oy });
        self.win[i].cx = asked.0;
        self.win[i].cy = asked.1;
        self.child_clamp(i, false);
        let (nx, ny) = (self.win[i].cx, self.win[i].cy);
        if (nx, ny) != (ox, oy) && self.win[i].surf != 0 {
            self.child_position(i);
            self.commit_parent(i);
            self.w.flush();
        }
        if (nx, ny) != asked {
            // kept inside the owner: the UI learns where it is
            self.inp.evwin = i as i32;
            self.push(E_WINPOS, 0, nx, ny, 0);
        }
    }

    /// keep child i inside its owner (its top-left corner; a child larger than the owner sits
    /// at 0, 0) and on the grid; report = E_WINPOS when it moved
    fn child_clamp(&mut self, i: usize, report: bool) -> bool {
        if self.win[i].owner < 0 {
            return false;
        }
        let o = slot(self.win[i].owner);
        let (ow, oh) = (self.win[o].pw, self.win[o].ph);
        if ow <= 0 || oh <= 0 {
            return false;
        }
        let r = self.win[i];
        let (mx, my) = ((ow - r.pw).max(0), (oh - r.ph).max(0));
        let nx = self.snap_px(o, r.cx.min(mx).max(0), 0, mx);
        let ny = self.snap_px(o, r.cy.min(my).max(0), 0, my);
        if (nx, ny) == (r.cx, r.cy) {
            return false;
        }
        self.win[i].cx = nx;
        self.win[i].cy = ny;
        if report {
            self.inp.evwin = i as i32;
            self.push(E_WINPOS, 0, nx, ny, 0);
        }
        true
    }

    /// (set_phys) window w changed its size: its children stay inside (the position applies
    /// with w's next present)
    pub(crate) fn children_clamp(&mut self, w: usize) {
        for c in 0..PW_MAX {
            if c != w && self.is_child(c as i32) && self.win[c].owner == w as i32 && self.win[c].surf != 0
                && self.child_clamp(c, true) {
                self.child_position(c);
            }
        }
    }

    /// (update_scale, before w's new size clamps them) window w went from scale old to its
    /// new one: its children keep their place in its content (px scale along), on the new grid
    pub(crate) fn children_rescale(&mut self, w: usize, old: i32) {
        let new = self.win[w].s120;
        if old <= 0 || old == new {
            return;
        }
        let sc = |v: i32| ((v as i64 * new as i64 + (old / 2) as i64) / old as i64) as i32;
        for c in 0..PW_MAX {
            if c != w && self.is_child(c as i32) && self.win[c].owner == w as i32 {
                let (x, y) = (self.win[c].cx, self.win[c].cy);
                self.win[c].cx = sc(x);
                self.win[c].cy = sc(y);
                if self.win[c].surf == 0 {
                    continue; // placed when shown
                }
                if self.drag.on && self.drag.w == c {
                    self.drag_stop(); // its numbers belong to the old scale
                }
                self.win[c].cx = self.snap_px(w, self.win[c].cx, 0, i32::MAX / 2);
                self.win[c].cy = self.snap_px(w, self.win[c].cy, 0, i32::MAX / 2);
                self.inp.evwin = c as i32;
                let (nx, ny) = (self.win[c].cx, self.win[c].cy);
                self.push(E_WINPOS, 0, nx, ny, 0);
                self.child_position(c);
            }
        }
    }

    /// the keys go to child c of the focused toplevel (-1 = the toplevel): E_FOCUS_OUT /
    /// E_FOCUS_IN when that changes who has them
    fn kb_child_set(&mut self, c: i32) {
        let before = self.kb_target();
        self.inp.kb_child = c;
        let after = self.kb_target();
        if after == before || !self.inp.kb_focus {
            return;
        }
        self.inp.evwin = before;
        self.push(E_FOCUS_OUT, 0, 0, 0, 0);
        self.inp.evwin = after;
        self.push(E_FOCUS_IN, 0, 0, 0, 0);
    }

    /// (win_activate) a child window gets the keys of its root and goes on top of its
    /// siblings; a toplevel takes them back from its children
    pub(crate) fn win_activate(&mut self, w: i32) {
        if !self.is_child(w) {
            let c = self.inp.kb_child;
            if c >= 0 && self.rec(w).is_some() && self.root_of(c as usize) as i32 == w {
                self.kb_child_set(-1);
            }
            return;
        }
        if self.win[w as usize].surf == 0 {
            return;
        }
        let i = w as usize;
        self.kb_child_set(w);
        let o = self.win[i].owner;
        // the sibling on top
        let mut top: Option<usize> = None;
        for c in 0..PW_MAX {
            if c == i || !self.is_child(c as i32) || self.win[c].owner != o || self.win[c].surf == 0 {
                continue;
            }
            if top.is_none_or(|t| self.win[c].z > self.win[t].z) {
                top = Some(c);
            }
        }
        let Some(t) = top else { return };
        if self.win[t].z < self.win[i].z {
            return;
        }
        let (sub, above) = (self.win[i].sub, self.win[t].surf);
        self.w.send(sub, &SUBSURFACE_PLACE_ABOVE, &[A::O(above)]);
        self.g.zc = self.g.zc.wrapping_add(1);
        self.win[i].z = self.g.zc;
        self.commit_parent(i);
        self.w.flush();
    }

    /// the window key events go to: the activated child of the focused toplevel, else it
    pub(crate) fn kb_target(&self) -> i32 {
        let c = self.inp.kb_child;
        if c >= 0 && self.is_child(c) && self.win[c as usize].surf != 0 && self.root_of(c as usize) as i32 == self.inp.kb_win {
            c
        } else {
            self.inp.kb_win
        }
    }

    /// (pointer press in window w at x, y px) click activation inside a toplevel: a child
    /// window takes the keys for a text box (not with PWF_NOACTIVATE), its root takes them back
    pub(crate) fn child_press(&mut self, w: i32, x: i32, y: i32) {
        if w < 0 || self.rec(w).is_none() {
            return;
        }
        if self.is_child(w) {
            if self.win[w as usize].flags & PWF_NOACTIVATE != 0 || crate::disp_hit(w, x, y) & HB_TEXT == 0 {
                return;
            }
            self.win_activate(w);
        } else if self.inp.kb_child >= 0 && self.root_of(self.inp.kb_child as usize) as i32 == w {
            self.kb_child_set(-1);
        }
    }

    // ------------------------------------------------------------------ move / resize

    /// start moving (edges 0) or resizing child i with the mouse button or pen tip that
    /// pressed on it (none held any more: nothing happens, as with a stale serial)
    pub(crate) fn child_drag_begin(&mut self, i: usize, edges: u32) {
        if self.win[i].surf == 0 || self.win[i].owner < 0 {
            return;
        }
        let held = self.held_buttons();
        let (tool, lx, ly) = if self.inp.ptr != 0 && held != 0 && self.inp.ptr_in && self.inp.ptr_win == i as i32 {
            (-1, self.inp.mx, self.inp.my)
        } else if let Some((k, x, y)) = self.tool_down_on(i) {
            (k as i32, x, y)
        } else {
            return;
        };
        if self.drag.on {
            self.drag_stop();
        }
        let r = self.win[i];
        let pos = [r.cx, r.cy];
        let model = self.drag.model;
        self.drag = Drag { on: true, w: i, edges, tool, btns: if tool < 0 { held } else { 0 }, start: [r.cx, r.cy, r.pw, r.ph],
                           grab: [r.cx + lx, r.cy + ly], applied: pos, goal: pos, sent: pos, model, ..Drag::default() };
        if tool < 0 {
            self.release_buttons(); // the press is the drag's now (as with xdg_toplevel.move)
            if model == MODEL_UNKNOWN && self.g.rpm != 0 && self.g.seat_ver >= 5 {
                let ptr = self.inp.ptr;
                self.drag.rp = self.w.send(self.g.rpm, &RPM_GET_RELATIVE_POINTER, &[A::N(&IFACE_ZWP_RP), A::O(ptr)]);
            }
        } else {
            self.tool_release(tool as usize);
        }
        self.w.flush();
    }

    /// the drag is over without a last move (its window went away)
    fn drag_stop(&mut self) {
        self.drag.on = false;
        self.drag.cb = 0;
        self.drag_rp_done();
    }

    /// the relative pointer is not needed any more
    fn drag_rp_done(&mut self) {
        if self.drag.rp != 0 {
            self.w.send(self.drag.rp, &RP_DESTROY, &[]);
            self.drag.rp = 0;
        }
        self.drag.lp = None;
    }

    /// (wl_pointer.motion of a mouse drag; x, y px in the child) handled with its frame
    /// (without frames, before wl_pointer version 5: at once)
    pub(crate) fn child_drag_ptr_motion(&mut self, x: i32, y: i32) {
        self.drag.cur = Some([x, y]);
        if self.g.seat_ver >= 5 {
            self.drag.lp = Some([x, y]);
        } else {
            let w = self.inp.ptr_win;
            self.child_drag_motion(w, x, y);
        }
    }

    /// (wl_pointer.frame of a mouse drag; wheel: it has wheel events) the model from this
    /// frame's motion, then the move.  A frame of a pointer motion without wl_pointer.motion
    /// is one whose position in the child did not change (mutter sends none then): the
    /// child moved along with the pointer, which is where it was plus that move
    pub(crate) fn child_drag_frame(&mut self, wheel: bool) {
        let d = self.drag;
        let rel = [self.ptr_px_of(d.w, d.rel[0]), self.ptr_px_of(d.w, d.rel[1])];
        let had = d.nrel != 0;
        self.drag.rel = [0; 2];
        self.drag.nrel = 0;
        let quiet = wheel || d.other; // a button or the wheel, not a motion
        self.drag.other = false;
        let w = self.inp.ptr_win;
        let Some(l) = self.drag.lp.take() else {
            self.drag.last_rel = false;
            if let Some(c) = d.cur.filter(|_| !quiet) {
                self.child_drag_motion(w, c[0], c[1]);
            }
            return;
        };
        if d.model == MODEL_UNKNOWN && had && d.last_rel {
            let j = d.jump;
            let k = if j[0].abs() >= j[1].abs() { 0 } else { 1 };
            if j[k].abs() >= 3 {
                let dl = l[k] - d.last[k];
                let e_cur = (dl - (rel[k] - j[k])).abs();
                let e_press = (dl - rel[k]).abs();
                let vote = if e_cur <= 2 && e_press >= 3 {
                    MODEL_CURRENT
                } else if e_press <= 2 && e_cur >= 3 {
                    MODEL_PRESS
                } else {
                    MODEL_UNKNOWN
                };
                if vote != MODEL_UNKNOWN {
                    if vote == d.vote {
                        self.drag.votes += 1;
                    } else {
                        self.drag.vote = vote;
                        self.drag.votes = 1;
                    }
                    if self.drag.votes >= 2 {
                        self.drag.model = vote;
                        self.drag_rp_done();
                    }
                }
            }
        }
        self.drag.last = l;
        self.drag.last_rel = had;
        self.drag.jump = [0; 2];
        self.child_drag_motion(w, l[0], l[1]);
    }

    /// (zwp_relative_pointer_v1.relative_motion during a drag) dx, dy 24.8
    pub(crate) fn child_drag_rel(&mut self, dx: i32, dy: i32) {
        self.drag.rel[0] = self.drag.rel[0].wrapping_add(dx);
        self.drag.rel[1] = self.drag.rel[1].wrapping_add(dy);
        self.drag.nrel += 1;
    }

    /// the pointer (or pen) is at x, y px of window win during a drag: the pointer in the
    /// owner is where the compositor's reference for the child is plus that
    pub(crate) fn child_drag_motion(&mut self, win: i32, x: i32, y: i32) {
        let d = self.drag;
        if win != d.w as i32 {
            return;
        }
        let base = if d.model == MODEL_PRESS { [d.start[0], d.start[1]] } else { d.applied };
        let dx = base[0] + x - d.grab[0];
        let dy = base[1] + y - d.grab[1];
        self.child_drag_apply(dx, dy);
    }

    /// the pointer moved dx, dy px since the drag began: the child's new rectangle (the size
    /// at once, the position when none is in flight), on the grid
    fn child_drag_apply(&mut self, dx: i32, dy: i32) {
        let d = self.drag;
        let i = d.w;
        if self.rec(i as i32).is_none() || self.win[i].surf == 0 || self.win[i].owner < 0 {
            self.drag_stop();
            return;
        }
        let [sx, sy, sw, sh] = d.start;
        let o = slot(self.win[i].owner);
        let (ow, oh) = (self.win[o].pw, self.win[o].ph);
        let (x0, y0, x1, y1);
        if d.edges == 0 {
            let mut rc = [sx + dx, sy + dy, sw, sh];
            crate::disp_moving(i as i32, &mut rc); // the UI's snap area (window_snap)
            // inside the owner, on the grid
            let (mx, my) = ((ow - sw).max(0), (oh - sh).max(0));
            x0 = self.snap_px(o, rc[0].min(mx).max(0), 0, mx);
            y0 = self.snap_px(o, rc[1].min(my).max(0), 0, my);
            x1 = x0 + sw;
            y1 = y0 + sh;
        } else {
            // the dragged edges follow (inside the owner, on the grid), the others stay
            let dpi = self.win[i].dpi.max(1);
            let minw = (self.win[i].minw * dpi / 96).max(16);
            let minh = (self.win[i].minh * dpi / 96).max(16);
            let (mut a0, mut b0, mut a1, mut b1) = (sx, sy, sx + sw, sy + sh);
            if d.edges & EDGE_LEFT != 0 {
                a0 = self.snap_px(o, (sx + dx).max(0).min(a1 - minw), 0, a1 - minw);
            }
            if d.edges & EDGE_RIGHT != 0 {
                a1 = self.snap_px(o, (a1 + dx).min(ow).max(a0 + minw), a0 + minw, ow.max(a0 + minw));
            }
            if d.edges & EDGE_TOP != 0 {
                b0 = self.snap_px(o, (sy + dy).max(0).min(b1 - minh), 0, b1 - minh);
            }
            if d.edges & EDGE_BOTTOM != 0 {
                b1 = self.snap_px(o, (b1 + dy).min(oh).max(b0 + minh), b0 + minh, oh.max(b0 + minh));
            }
            (x0, y0, x1, y1) = (a0, b0, a1, b1);
        }
        let r = self.win[i];
        let (nw, nh) = (x1 - x0, y1 - y0);
        if (nw, nh) != (r.pw, r.ph) {
            self.win[i].pw = nw;
            self.win[i].ph = nh;
            self.win[i].lw = self.logical_of(i, nw);
            self.win[i].lh = self.logical_of(i, nh);
            self.win[i].normal_w = self.win[i].lw;
            self.win[i].normal_h = self.win[i].lh;
            self.win[i].resize_pending = true;
            self.inp.evwin = i as i32;
            self.push(E_RESIZE, 0, nw, nh, 0);
        }
        self.drag.goal = [x0, y0];
        self.child_drag_send();
    }

    /// the wanted position goes out unless one is in flight: set_position, the owner's
    /// commit, a sync whose answer tells that the compositor has it
    fn child_drag_send(&mut self) {
        let d = self.drag;
        let i = d.w;
        if d.cb != 0 || d.goal == [self.win[i].cx, self.win[i].cy] || self.win[i].surf == 0 {
            return;
        }
        self.win[i].cx = d.goal[0];
        self.win[i].cy = d.goal[1];
        self.child_position(i);
        self.commit_parent(i);
        if self.drag.on {
            let cb = self.w.send(1, &DISPLAY_SYNC, &[A::N(&IFACE_CB_DRAG)]);
            self.drag.cb = cb;
            self.drag.sent = d.goal;
        } else {
            self.drag.applied = d.goal;
        }
        self.w.flush();
        self.inp.evwin = i as i32;
        self.push(E_WINPOS, 0, d.goal[0], d.goal[1], 0);
    }

    /// the button was released (or the pointer left, the pen went up): the drag ends (the
    /// last position goes out)
    pub(crate) fn child_drag_end(&mut self) {
        if self.drag.lp.is_some() {
            let l = self.drag.lp.take().unwrap_or_default();
            let w = self.inp.ptr_win;
            self.child_drag_motion(w, l[0], l[1]);
        }
        self.drag.on = false;
        self.drag.cb = 0;
        self.drag_rp_done();
        self.child_drag_send();
    }
}

/// wl_callback.done of the sync after a child window's new position: the compositor has it
pub(crate) fn wlh_drag_done(wl: &mut Wl, id: u32, _a: &Args, _u: u64) {
    if wl.drag.cb != id {
        return;
    }
    let d = wl.drag;
    wl.drag.cb = 0;
    wl.drag.jump = [d.jump[0] + d.sent[0] - d.applied[0], d.jump[1] + d.sent[1] - d.applied[1]];
    wl.drag.applied = d.sent;
    if wl.drag.on {
        wl.child_drag_send(); // a newer one waited
    }
}

/// zwp_relative_pointer_v1.relative_motion(utime_hi, utime_lo, dx, dy, dx_unaccel,
/// dy_unaccel): the pointer's motion during a mouse drag (finding out the model)
pub(crate) fn wlh_rel_motion(wl: &mut Wl, id: u32, a: &Args, _u: u64) {
    if wl.drag.on && wl.drag.rp == id {
        wl.child_drag_rel(a.i(2), a.i(3));
    }
}
