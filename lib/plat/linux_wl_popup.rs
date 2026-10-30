// linux_wl_popup -- OS popup windows on Wayland (U3)                [src/plat/linux/wl_popup.s]
//
//   Wl::popup_anchor(slot, anchor, kind, nat)  (§6) how the next popup() of the slot hangs at
//        its anchor (owner px): the positioner below uses it; without it the anchor point is
//        rect.x, rect.y (PA_FREE)
//   Wl::popup(slot, owner, rect, frame)
//        an xdg_popup of the owner's xdg_surface (of its toplevel for a child window: the
//        positions are moved by the child's offset).  Logical = px / the toplevel's scale:
//        with a fractional scale (wp_fractional_scale) the buffer keeps the frame's pixels and a
//        wp_viewport gives its logical size (integer scales: wl_surface.set_buffer_scale).
//        Positioner: the anchor rectangle, anchor edge and gravity of the PA_* kind (below /
//        above the anchor with the gap in the anchor rectangle, beside it, at a point), the
//        offset to the UI's natural place, flips and slides (the compositor keeps it on the
//        screen: below a button when there is room, above it when not); no grab (the UI
//        dismisses its popups itself).  rect is where the UI drew the content: the pointer
//        offsets follow it.  The same anchor, kind, natural place and size keep the popup (the
//        UI moved it to where the compositor put it, see popup_placed); anything else creates
//        it anew (xdg_popup.reposition needs version 3).  The content goes into two wl_shm
//        buffers of one pool per slot (the one the compositor does not hold).  rect None
//        destroys the popup's objects (WSLg: only a destroyed wl_surface removes its RAIL
//        window).  The frame's pixels are kept (the assembly keeps the caller's surface record).
//   Wl::popup_placed(slot) -> where the compositor put the popup (owner px; xdg_popup.configure)
//        when that is not its natural place: the UI lays the popup out there (an E_WAKE asks
//        for the frame); None: as asked, or not configured yet
//   Wl::popup_ptr(surface) -> owner index (-1 = not a popup): pointer enter;
//        inp.off_x / off_y = the popup's offset in the owner (physical px), added to every
//        pointer position until the pointer enters another surface (popup_ptr_reset)
//   wlh_pop_xdg_configure / wlh_pop_configure / wlh_pop_done / wlh_pop_release  handlers

use alloc::vec::Vec;

use rt::os::{syscall6, SYS_MUNMAP};

use super::proto::*;
use super::wire::{Args, A};
use super::Wl;
use crate::{E_WAKE, Event, Frame, PA_ABOVE, PA_BELOW, PA_POINT, PA_SIDE, PPW_MAX, evq_push};

const WQ_MAX: usize = PPW_MAX as usize;
const SHM_XRGB: u32 = 1;
// xdg_positioner anchor / gravity values and constraint adjustments
const POS_TOP_LEFT: u32 = 5;
const POS_BOTTOM_LEFT: u32 = 6;
const POS_TOP_RIGHT: u32 = 7;
const POS_BOTTOM_RIGHT: u32 = 8;
const POS_SLIDE_X: u32 = 1;
const POS_SLIDE_Y: u32 = 2;
const POS_FLIP_X: u32 = 4;
const POS_FLIP_Y: u32 = 8;

/// a popup slot
#[derive(Default)]
pub(crate) struct Wq {
    surf: u32, // wl_surface (0 = no popup objects)
    xsurf: u32,
    popup: u32,
    owner: i32,    // owner platform window (-1 = hidden)
    rect: [i32; 4], // popup pixel 0, 0 = owner pixel dx, dy; size (px)
    scale: i32,    // integer buffer scale (the toplevel's)
    s120: i32,     // the toplevel's scale (120ths)
    frac: bool,    // fractional: a viewport, no buffer scale
    vp: u32,       // wp_viewport (fractional)
    vdst: [i32; 2], // its destination size sent
    vsrc: [i32; 2], // its source size sent (24.8)
    off: [i32; 2], // the owner's offset in its toplevel (child windows; px)
    conf: bool,    // its first configure was acknowledged
    pool: u32,     // wl_shm_pool (0 = none)
    cap: u32,      // pool bytes (two buffers)
    map: usize,
    buf: [u32; 2],
    busy: [bool; 2], // held by the compositor
    bw: i32,         // buffer size (px)
    bh: i32,
    sf: Option<(Vec<u8>, i32, i32)>, // the frame to show (rows of w * 4 bytes), w, h
    // (§6) the positioner of the popup: anchor (owner px), PA_* kind, the UI's natural place
    anchor: [i32; 4],
    kind: u32,
    nat: [i32; 2],
    next: Option<([i32; 4], u32, [i32; 2])>, // popup_anchor of the next popup()
    placed: Option<[i32; 2]>,                 // where the configure put it (owner px)
    ptr: bool,                                // the pointer is on it (inp.off_x / off_y)
}

impl Wq {
    /// px -> logical units of the toplevel (rounded)
    fn lg(&self, v: i32) -> i32 {
        if self.frac {
            let s = self.s120.max(1) as i64;
            let n = v as i64 * 120;
            (if n >= 0 { (n + s / 2) / s } else { (n - s / 2) / s }) as i32
        } else {
            v.wrapping_div(self.scale.max(1))
        }
    }

    /// a size in px -> the logical size it covers (fractional: rounded down, the viewport's
    /// source shows it pixel for pixel)
    fn lgs(&self, v: i32) -> i32 {
        if self.frac { (v.max(0) as i64 * 120 / self.s120.max(1) as i64).max(1) as i32 } else { self.lg(v).max(1) }
    }

    /// logical units of the toplevel -> px
    fn ph(&self, l: i32) -> i32 {
        if self.frac {
            let n = l as i64 * self.s120 as i64;
            (if n >= 0 { (n + 60) / 120 } else { (n - 60) / 120 }) as i32
        } else {
            l.wrapping_mul(self.scale.max(1))
        }
    }
}

/// a copy of the frame
fn keep(f: &Frame) -> (Vec<u8>, i32, i32) {
    let (w, h) = (f.w.max(0) as usize, f.h.max(0) as usize);
    let mut v = alloc::vec![0u8; w * h * 4];
    for y in 0..h {
        let s = y * f.stride as usize;
        if let Some(src) = f.px.get(s..s + w * 4) {
            v[y * w * 4..(y + 1) * w * 4].copy_from_slice(src);
        }
    }
    (v, w as i32, h as i32)
}

impl Wl {
    /// (§6) how the next popup() of the slot hangs at its anchor
    pub(crate) fn popup_anchor(&mut self, slot: u32, anchor: [i32; 4], kind: u32, nat: [i32; 2]) {
        if let Some(q) = self.wq.get_mut(slot as usize) {
            q.next = Some((anchor, kind, nat));
        }
    }

    /// (§6) where the compositor put the slot's popup when that is not its natural place
    pub(crate) fn popup_placed(&mut self, slot: u32) -> Option<[i32; 2]> {
        let q = self.wq.get(slot as usize)?;
        q.placed.filter(|p| q.surf != 0 && *p != q.nat)
    }

    /// show / move / present popup slot `slot` over window `owner`  [wl_plat_popup]
    pub(crate) fn popup(&mut self, slot: u32, owner: i32, rect: Option<[i32; 4]>, f: Option<&Frame>) {
        if slot as usize >= WQ_MAX {
            return;
        }
        let k = slot as usize;
        let next = self.wq[k].next.take();
        let Some(rect) = rect else {
            self.wq[k].owner = -1;
            self.wq[k].sf = None;
            self.wq_objects_destroy(k);
            self.w.flush();
            return;
        };
        // the positioner: the UI's anchor, else the anchor point at rect.x, rect.y
        let (anchor, kind, nat) = next.unwrap_or(([rect[0], rect[1], 1, 1], 0, [rect[0], rect[1]]));
        let Some(o) = self.rec(owner) else { return };
        // the xdg_surface the popup hangs at: the owner's, a child window's toplevel's
        let root = self.root_of(o);
        if self.win[root].xsurf == 0 || self.win[o].surf == 0 {
            return; // the owner is not shown
        }
        let (ox, oy) = self.child_offset(o);
        // the toplevel's scale: fractional with a viewport, else an integer buffer scale
        let rr = self.win[root];
        let s120 = rr.s120.max(1);
        // a viewport whenever there is one: also at an integer scale (a buffer scale would need
        // sizes that are multiples of it -- else wl_surface.invalid_size ends the connection)
        let frac = self.g.vpr != 0;
        let s = if frac { 1 } else { ((s120 as u32).wrapping_add(60) / 120).max(1) as i32 };
        let q = &self.wq[k];
        let same = q.surf != 0 && q.owner == owner && q.scale == s && q.s120 == s120 && q.frac == frac
            && q.off == [ox, oy] && q.rect[2] == rect[2] && q.rect[3] == rect[3];
        if same && q.anchor == anchor && q.kind == kind && q.nat == nat {
            // the same popup (the UI may have moved its content to where the compositor put it)
            if self.w.debug && rect != self.wq[k].rect {
                crate::write_stderr(alloc::format!("[wl] popup {k}: content at {}, {} (natural {}, {})\n", rect[0], rect[1], nat[0], nat[1]).as_bytes());
            }
            if self.wq[k].ptr {
                self.inp.off_x = self.inp.off_x.wrapping_add(rect[0] - self.wq[k].rect[0]);
                self.inp.off_y = self.inp.off_y.wrapping_add(rect[1] - self.wq[k].rect[1]);
            }
            self.wq[k].rect = rect;
            self.wq[k].sf = f.map(keep);
            if self.wq[k].conf {
                self.wq_draw(k);
            }
        } else {
            // another anchor / place / size / owner / scale: a new popup
            if q.surf != 0 {
                self.wq_objects_destroy(k);
            }
            let q = &mut self.wq[k];
            q.owner = owner;
            q.scale = s;
            q.s120 = s120;
            q.frac = frac;
            q.off = [ox, oy];
            q.rect = rect;
            q.anchor = anchor;
            q.kind = kind;
            q.nat = nat;
            q.sf = f.map(keep);
            self.wq_objects_create(k, root);
        }
        self.w.flush();
    }

    /// surface, xdg_surface, positioner, xdg_popup; the initial commit (the content follows
    /// the configure)  [wq_objects_create]
    fn wq_objects_create(&mut self, k: usize, owner: usize) {
        let ku = k as u64;
        self.wq[k].conf = false;
        let surf = self.w.send(self.g.comp, &COMPOSITOR_CREATE_SURFACE, &[A::N(&IFACE_POP_SURFACE)]);
        self.wq[k].surf = surf;
        self.w.set_udata(surf, ku);
        let xs = self.w.send(self.g.xdg_wm, &WM_BASE_GET_XDG_SURFACE, &[A::N(&IFACE_POP_XSURF), A::O(surf)]);
        self.wq[k].xsurf = xs;
        self.w.set_udata(xs, ku);
        // positioner: size, anchor rectangle, edge, gravity, offset in logical coordinates of
        // the owner (wq_place)
        let pos = self.w.send(self.g.xdg_wm, &WM_BASE_CREATE_POSITIONER, &[A::N(&IFACE_XDG_POSITIONER)]);
        let s = self.wq[k].scale;
        let r = self.wq[k].rect;
        let q = &self.wq[k];
        let lg = |v: i32| q.lg(v);
        let (ar, edge, grav, off, adj) = wq_place(q);
        let [dx, dy] = q.off; // a child window's offset in its toplevel
        let size = [q.lgs(r[2]), q.lgs(r[3])];
        let mut arl = [lg(ar[0] + dx), lg(ar[1] + dy), lg(ar[2]).max(1), lg(ar[3]).max(1)];
        {
            // the anchor rectangle may not extend outside the toplevel's window geometry
            // (xdg_positioner): a child larger than its owner, a widget at the edge with the
            // gap of PA_BELOW / PA_ABOVE reach beyond it
            let (gw, gh) = (self.win[owner].lw.max(1), self.win[owner].lh.max(1));
            let x0 = arl[0].clamp(0, gw - 1);
            let y0 = arl[1].clamp(0, gh - 1);
            arl = [x0, y0, (arl[0] + arl[2]).clamp(x0 + 1, gw) - x0, (arl[1] + arl[3]).clamp(y0 + 1, gh) - y0];
        }
        let offl = [lg(off[0]), lg(off[1])];
        self.wq[k].placed = None;
        self.w.send(pos, &POSITIONER_SET_SIZE, &[A::I(size[0]), A::I(size[1])]);
        self.w.send(pos, &POSITIONER_SET_ANCHOR_RECT, &[A::I(arl[0]), A::I(arl[1]), A::I(arl[2]), A::I(arl[3])]);
        self.w.send(pos, &POSITIONER_SET_ANCHOR, &[A::U(edge)]);
        self.w.send(pos, &POSITIONER_SET_GRAVITY, &[A::U(grav)]);
        if off != [0, 0] {
            self.w.send(pos, &POSITIONER_SET_OFFSET, &[A::I(offl[0]), A::I(offl[1])]);
        }
        self.w.send(pos, &POSITIONER_SET_CONSTRAINT_ADJUSTMENT, &[A::U(adj)]);
        let parent = self.win[owner].xsurf;
        let p = self.w.send(xs, &XDG_SURFACE_GET_POPUP, &[A::N(&IFACE_XDG_POPUP), A::O(parent), A::O(pos)]);
        self.wq[k].popup = p;
        self.w.set_udata(p, ku);
        self.w.send(pos, &POSITIONER_DESTROY, &[]);
        self.wq[k].vdst = [0, 0];
        self.wq[k].vsrc = [0, 0];
        if self.wq[k].frac {
            // the buffer keeps the frame's pixels, the viewport gives its logical size
            let vp = self.w.send(self.g.vpr, &VIEWPORTER_GET_VIEWPORT, &[A::N(&IFACE_WP_VIEWPORT), A::O(surf)]);
            self.wq[k].vp = vp;
        } else if s as u32 > 1 {
            self.w.send(surf, &SURFACE_SET_BUFFER_SCALE, &[A::I(s)]);
        }
        self.w.send(surf, &SURFACE_COMMIT, &[]);
    }

    /// popup, xdg_surface, surface (the buffers stay for reuse)  [wq_objects_destroy]
    fn wq_objects_destroy(&mut self, k: usize) {
        let q = &self.wq[k];
        let (p, x, s, vp) = (q.popup, q.xsurf, q.surf, q.vp);
        if p != 0 {
            self.w.send(p, &POPUP_DESTROY, &[]);
        }
        if x != 0 {
            self.w.send(x, &XDG_SURFACE_DESTROY, &[]);
        }
        if vp != 0 {
            self.w.send(vp, &VIEWPORT_DESTROY, &[]);
        }
        if s != 0 {
            self.w.send(s, &SURFACE_DESTROY, &[]);
        }
        let q = &mut self.wq[k];
        q.popup = 0;
        q.xsurf = 0;
        q.surf = 0;
        q.vp = 0;
        q.conf = false;
    }

    /// the frame into a free buffer: attach, damage, commit  [wq_draw]
    fn wq_draw(&mut self, k: usize) {
        let q = &self.wq[k];
        let Some((_, sw, sh)) = q.sf.as_ref() else { return };
        if q.surf == 0 {
            return;
        }
        let cw = q.rect[2].min(*sw);
        let ch = q.rect[3].min(*sh);
        if cw <= 0 || ch <= 0 {
            return;
        }
        // with a buffer scale (no viewporter) the buffer's size must be a multiple of it: the
        // frame padded with transparent pixels
        let bs = if q.frac { 1 } else { q.scale.max(1) };
        let (w, h) = ((cw + bs - 1) / bs * bs, (ch + bs - 1) / bs * bs);
        // two buffers of w x h in one pool, offsets int32 (wl_shm): below 2 GiB
        if w as u64 * h as u64 * 8 > i32::MAX as u64 {
            return;
        }
        // buffers of this size in a pool large enough for two
        if (q.bw != w || q.bh != h || q.buf[0] == 0) && !self.wq_buffers(k, w, h) {
            return;
        }
        let q = &mut self.wq[k];
        let b = if !q.busy[0] {
            0
        } else if !q.busy[1] {
            1
        } else {
            return; // both held: the next update draws
        };
        let Some((px, sw, _)) = q.sf.as_ref() else { return };
        let row = w as usize * 4;
        let crow = cw as usize * 4;
        let base = w as usize * h as usize * 4 * b;
        // SAFETY: the pool holds two w x h buffers from q.map (wq_buffers sized it).
        let dst = unsafe { core::slice::from_raw_parts_mut((q.map + base) as *mut u8, row * h as usize) };
        if (w, h) != (cw, ch) {
            dst.fill(0);
        }
        for y in 0..ch as usize {
            let s = y * *sw as usize * 4;
            if let Some(src) = px.get(s..s + crow) {
                dst[y * row..y * row + crow].copy_from_slice(src);
            }
        }
        q.busy[b] = true;
        let (surf, buf) = (q.surf, q.buf[b]);
        // the logical size the buffer covers, shown pixel for pixel (the source: exactly that)
        let s = q.s120.max(1);
        let dst = [q.lgs(w), q.lgs(h)];
        if q.vp != 0 && dst != q.vdst {
            let vp = q.vp;
            q.vdst = dst;
            let src = super::present::viewport_src(dst[0], dst[1], s, w, h);
            if src != q.vsrc {
                q.vsrc = src;
                self.w.send(vp, &VIEWPORT_SET_SOURCE, &[A::I(0), A::I(0), A::I(src[0]), A::I(src[1])]);
            }
            self.w.send(vp, &VIEWPORT_SET_DESTINATION, &[A::I(dst[0]), A::I(dst[1])]);
        }
        self.w.send(surf, &SURFACE_ATTACH, &[A::O(buf), A::I(0), A::I(0)]);
        self.w.send(surf, &SURFACE_DAMAGE_BUFFER, &[A::I(0), A::I(0), A::I(w), A::I(h)]);
        self.w.send(surf, &SURFACE_COMMIT, &[]);
    }

    /// two w x h buffers (a new pool when it is small) -> ok  [wq_buffers]
    fn wq_buffers(&mut self, k: usize, w: i32, h: i32) -> bool {
        // the old buffers go (a held one is released by the compositor later: harmless)
        for i in 0..2 {
            let b = self.wq[k].buf[i];
            if b != 0 {
                self.w.send(b, &BUFFER_DESTROY, &[]);
                self.wq[k].buf[i] = 0;
                self.wq[k].busy[i] = false;
            }
        }
        self.wq[k].bw = 0;
        self.wq[k].bh = 0;
        // two buffers (wq_draw keeps them below 2 GiB: no overflow here)
        let bytes = (w as u64 * h as u64 * 8).min(i32::MAX as u64) as u32;
        if bytes > self.wq[k].cap {
            // a larger pool (the old one is destroyed: its buffers are gone)
            let (pool, map, cap) = (self.wq[k].pool, self.wq[k].map, self.wq[k].cap);
            if pool != 0 {
                self.w.send(pool, &SHM_POOL_DESTROY, &[]);
                self.wq[k].pool = 0;
                // SAFETY: the old pool's mapping.
                unsafe { syscall6(SYS_MUNMAP, map, cap as usize, 0, 0, 0, 0) };
            }
            let cap = ((bytes as u64 + 65535) & !65535) as u32;
            self.wq[k].cap = cap;
            match self.shm_pool_new(cap as usize) {
                Some((p, m)) => {
                    self.wq[k].pool = p;
                    self.wq[k].map = m;
                }
                None => {
                    self.wq[k].cap = 0;
                    return false;
                }
            }
        }
        let pool = self.wq[k].pool;
        for i in 0..2u32 {
            let off = w as u32 * h as u32 * 4 * i;
            let b = self.w.send(pool, &SHM_POOL_CREATE_BUFFER,
                                &[A::N(&IFACE_POP_BUFFER), A::I(off as i32), A::I(w), A::I(h), A::I(w * 4),
                                  A::U(SHM_XRGB)]);
            self.wq[k].buf[i as usize] = b;
            self.w.set_udata(b, (k * 2) as u64 + i as u64); // udata: slot * 2 + buffer
        }
        self.wq[k].bw = w;
        self.wq[k].bh = h;
        true
    }

    /// the owner of the popup with this surface (-1 = not a popup); its offset in
    /// inp.off_x / off_y  [wl_popup_ptr]
    pub(crate) fn popup_ptr(&mut self, surface: u32) -> i32 {
        if surface == 0 {
            return -1;
        }
        match self.wq.iter().position(|q| q.surf == surface) {
            Some(k) => {
                self.inp.off_x = self.wq[k].rect[0];
                self.inp.off_y = self.wq[k].rect[1];
                self.wq[k].ptr = true;
                self.wq[k].owner
            }
            None => -1,
        }
    }

    /// (pointer enter) the pointer is on none of the popups
    pub(crate) fn popup_ptr_reset(&mut self) {
        for q in self.wq.iter_mut() {
            q.ptr = false;
        }
    }
}

// the positioner of a popup from its anchor and kind -> anchor rectangle (owner px), anchor edge,
// gravity, offset (px), constraint adjustment.  The gap between anchor and popup becomes part
// of the anchor rectangle, so a flip keeps it (the offset is not flipped).
fn wq_place(q: &Wq) -> ([i32; 4], u32, u32, [i32; 2], u32) {
    let a = [q.anchor[0], q.anchor[1], q.anchor[2].max(1), q.anchor[3].max(1)];
    let (w, h) = (q.rect[2], q.rect[3]);
    let [nx, ny] = q.nat;
    match q.kind {
        PA_BELOW | PA_ABOVE => {
            let below = ny >= a[1] + a[3];
            let gap = if below { ny - (a[1] + a[3]) } else { a[1] - (ny + h) }.max(0);
            let r = [a[0], a[1] - gap, a[2], a[3] + 2 * gap];
            let (edge, grav) = if below { (POS_BOTTOM_LEFT, POS_BOTTOM_RIGHT) } else { (POS_TOP_LEFT, POS_TOP_RIGHT) };
            (r, edge, grav, [nx - a[0], 0], POS_FLIP_Y | POS_SLIDE_X | POS_SLIDE_Y)
        }
        PA_SIDE => {
            let right = nx + w > a[0] + a[2];
            let (edge, grav) = if right { (POS_TOP_RIGHT, POS_BOTTOM_RIGHT) } else { (POS_TOP_LEFT, POS_BOTTOM_LEFT) };
            (a, edge, grav, [0, ny - a[1]], POS_FLIP_X | POS_SLIDE_X | POS_SLIDE_Y)
        }
        PA_POINT => {
            let grav = match (nx >= a[0], ny >= a[1]) {
                (true, true) => POS_BOTTOM_RIGHT,
                (true, false) => POS_TOP_RIGHT,
                (false, true) => POS_BOTTOM_LEFT,
                (false, false) => POS_TOP_LEFT,
            };
            ([a[0], a[1], 1, 1], POS_TOP_LEFT, grav, [0, 0], POS_FLIP_X | POS_FLIP_Y | POS_SLIDE_X | POS_SLIDE_Y)
        }
        _ => ([nx, ny, 1, 1], POS_TOP_LEFT, POS_BOTTOM_RIGHT, [0, 0], POS_SLIDE_X | POS_SLIDE_Y),
    }
}

// ------------------------------------------------------------------ events

/// xdg_surface.configure: ack, then the content  [wlh_pop_xdg_configure]
pub(crate) fn wlh_pop_xdg_configure(wl: &mut Wl, _id: u32, a: &Args, u: u64) {
    let k = u as usize & (WQ_MAX - 1);
    let xs = wl.wq[k].xsurf;
    if xs == 0 {
        return;
    }
    wl.w.send(xs, &XDG_SURFACE_ACK_CONFIGURE, &[A::U(a.u(0))]);
    wl.wq[k].conf = true;
    wl.wq_draw(k);
    wl.w.flush();
}

// where the positioner of wq_place puts the popup when nothing constrains it: its top-left in
// logical coordinates (the anchor point of the edge, the gravity's corner, the offset)
fn wq_unmoved(q: &Wq) -> [i32; 2] {
    let lg = |v: i32| q.lg(v);
    let (r, edge, grav, off, _) = wq_place(q);
    let (rw, rh) = (lg(r[2]).max(1), lg(r[3]).max(1));
    let (w, h) = (q.lgs(q.rect[2]), q.lgs(q.rect[3]));
    let (mut x, mut y) = (lg(r[0] + q.off[0]), lg(r[1] + q.off[1]));
    if matches!(edge, POS_TOP_RIGHT | POS_BOTTOM_RIGHT) {
        x += rw;
    }
    if matches!(edge, POS_BOTTOM_LEFT | POS_BOTTOM_RIGHT) {
        y += rh;
    }
    if matches!(grav, POS_TOP_LEFT | POS_BOTTOM_LEFT) {
        x -= w;
    }
    if matches!(grav, POS_TOP_LEFT | POS_TOP_RIGHT) {
        y -= h;
    }
    [x + lg(off[0]), y + lg(off[1])]
}

/// xdg_popup.configure(x, y, w, h): where the compositor put the popup (logical, relative to the
/// owner's window geometry); not its natural place: the UI is asked for a frame that lays the
/// popup out there (popup_placed)
pub(crate) fn wlh_pop_configure(wl: &mut Wl, _id: u32, a: &Args, u: u64) {
    let k = u as usize & (WQ_MAX - 1);
    let q = &mut wl.wq[k];
    let p = [a.i(0), a.i(1)];
    // where the positioner puts it without constraints (logical): the natural place, else
    // where it is in the owner (px; a child window's offset taken off).  With a fractional
    // scale the natural place is only reachable to a pixel: the content follows the popup
    let at = [q.ph(p[0]) - q.off[0], q.ph(p[1]) - q.off[1]];
    let placed = if p == wq_unmoved(q) && (!q.frac || at == q.nat) { q.nat } else { at };
    let moved = q.placed != Some(placed) && placed != q.nat;
    q.placed = Some(placed);
    if moved && q.owner >= 0 {
        evq_push(Event { typ: E_WAKE, win: q.owner as u8, ..Default::default() });
    }
}

/// the compositor dismissed the popup: its objects go (the UI closes it with the next
/// press anyway)  [wlh_pop_done]
pub(crate) fn wlh_pop_done(wl: &mut Wl, _id: u32, _a: &Args, u: u64) {
    wl.wq_objects_destroy(u as usize & (WQ_MAX - 1));
    wl.w.flush();
}

/// wl_buffer.release (udata slot * 2 + buffer)  [wlh_pop_release]
pub(crate) fn wlh_pop_release(wl: &mut Wl, _id: u32, _a: &Args, u: u64) {
    wl.wq[(u as usize >> 1) & (WQ_MAX - 1)].busy[u as usize & 1] = false;
}
