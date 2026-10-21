// linux_wl_present -- shm buffers and frame presentation (Wayland), per window
//                                                                [src/plat/linux/wl_present.s]
//
//   Wl::win_present(w, frame, rects)   rects = x, y, w, h; None = all.  The frame's pixels
//        (B, G, R, x rows) are kept in the window's own copy (the assembly keeps the caller's
//        surface record: in Rust the frame is borrowed only for the call).  Dirty areas are
//        copied into one of three wl_shm buffers of the window (XRGB8888 == our BGRX memory,
//        no swizzle), then attached + damaged + committed.  While a frame callback is
//        outstanding the damage is accumulated and presented when wl_callback.done arrives
//        (or after 1 s, e.g. while minimized; only one callback is ever outstanding per
//        window).  Before the window's first configure (a window just shown) the damage
//        waits for it.
//   Wl::present_service(now) -> next deadline (u64::MAX none)      (wait)
//   Wl::present_unmapped(w)            the window was unmapped
//   Wl::present_free(w)                the window is destroyed: its buffers and pools
//   Wl::shm_pool_new(bytes) -> (wl_shm_pool, mapping)   memfd + mmap
//   wlh_buffer_release / wlh_frame_done    event handlers (udata = window)
//
// Every slot remembers the area that changed since it was last drawn ("stale" box), so a
// slot that comes back from the compositor gets that area too.  Growing beyond the pool
// creates a new pool (slot = max(1.25 x need, largest output mode; 256 KB for secondary
// windows)); buffers of the old pool that are still busy are destroyed on release and the
// old mapping is unmapped when none remain.  A window mapped again (shown after a hide) gets
// its last frame when it is configured.

use alloc::vec;
use alloc::vec::Vec;

use rt::os::{syscall3, syscall6, SYS_FTRUNCATE, SYS_MEMFD_CREATE, SYS_MMAP, SYS_MUNMAP};

use super::proto::*;
use super::wire::{sys_close, Args, A, EINTR};
use super::window::WLWF_TOP;
use super::Wl;
use crate::{Frame, PW_MAX};

const SLOT_N: usize = 3;
const POOL_N: usize = 4;
const ORPH_N: usize = 8; // busy buffers of retired pools {buffer, pool}
const PR_MAX: usize = 16; // pending rectangles (x0, y0, x1, y1)
const FRAME_SAFETY: u64 = 1_000_000; // us to wait for wl_callback.done
const SHM_XRGB8888: u32 = 1;
const SLOT_MIN_SECONDARY: u64 = 262144; // smallest slot of a secondary window

#[derive(Clone, Copy, Default)]
struct Slot {
    buf: u32, // wl_buffer (0 = none)
    w: i32,
    h: i32,
    busy: bool, // attached, not yet released
    pool: u32,  // pool index + 1
    st: [i32; 4], // stale box (empty when x0 >= x1)
    mem: usize,   // pixels in our mapping
}

#[derive(Clone, Copy, Default)]
struct Pool {
    map: usize,
    len: usize,
    id: u32,   // wl_shm_pool (0 = destroyed)
    nbuf: u32, // live wl_buffers created from it
    slot: u32, // bytes per slot
    used: bool,
}

/// the presentation record of a window
#[derive(Default)]
pub(crate) struct Wp {
    sf: Vec<u8>, // the last presented frame (rows of sf_w * 4 bytes)
    sf_w: i32,
    sf_h: i32,
    sfok: bool,
    pend: bool, // damage waits to be presented
    prn: usize,
    over: bool,
    bx: [i32; 4], // bounding box (x0, y0, x1, y1)
    pr: [[i32; 4]; PR_MAX],
    frame_cb: u32,    // outstanding wl_callback (0 = none)
    frame_wait: bool, // presents wait for it (or FRAME_SAFETY)
    frame_t0: u64,
    cur_pool: u32,
    sent_scale: i32,
    sent_gw: i32,
    sent_gh: i32,
    sent_vw: i32,
    sent_vh: i32,
    slots: [Slot; SLOT_N],
    pools: [Pool; POOL_N],
    orph: [(u32, u32); ORPH_N],
}

impl Wp {
    /// add a (clipped, non-empty) box to the pending damage  [pr_add]
    fn pr_add(&mut self, b: [i32; 4]) {
        if self.bx[0] < self.bx[2] {
            self.bx[0] = self.bx[0].min(b[0]);
            self.bx[1] = self.bx[1].min(b[1]);
            self.bx[2] = self.bx[2].max(b[2]);
            self.bx[3] = self.bx[3].max(b[3]);
        } else {
            self.bx = b;
        }
        if self.prn < PR_MAX {
            self.pr[self.prn] = b;
            self.prn += 1;
        } else {
            self.over = true;
        }
    }

    /// copy the frame's box into the kept copy
    fn keep(&mut self, f: &Frame, b: [i32; 4]) {
        let n = (b[2] - b[0]) as usize * 4;
        for y in b[1]..b[3] {
            let s = y as usize * f.stride as usize + b[0] as usize * 4;
            let d = (y as usize * self.sf_w as usize + b[0] as usize) * 4;
            if let (Some(src), Some(dst)) = (f.px.get(s..s + n), self.sf.get_mut(d..d + n)) {
                dst.copy_from_slice(src);
            }
        }
    }
}

/// dst |= src (empty boxes ignored)  [box_union]
fn box_union(d: &mut [i32; 4], s: &[i32; 4]) {
    if s[0] >= s[2] {
        return;
    }
    if d[0] >= d[2] {
        *d = *s;
        return;
    }
    d[0] = d[0].min(s[0]);
    d[1] = d[1].min(s[1]);
    d[2] = d[2].max(s[2]);
    d[3] = d[3].max(s[3]);
}

/// copy a box of the kept frame into the slot (clipped)  [copy_box]
fn copy_box(sl: &Slot, b: &[i32; 4], sf: &[u8], sf_w: i32) {
    let x0 = b[0].max(0);
    let y0 = b[1].max(0);
    let x1 = b[2].min(sl.w);
    let y1 = b[3].min(sl.h);
    if x1 - x0 <= 0 {
        return;
    }
    let n = (x1 - x0) as usize * 4;
    // SAFETY: the slot's buffer lies in our mapping, sl.w * sl.h pixels from sl.mem (the
    // pool's slot size is at least that).
    let dst = unsafe { core::slice::from_raw_parts_mut(sl.mem as *mut u8, sl.w as usize * sl.h as usize * 4) };
    for y in y0..y1 {
        let d = (y as usize * sl.w as usize + x0 as usize) * 4;
        let s = (y as usize * sf_w as usize + x0 as usize) * 4;
        if let Some(src) = sf.get(s..s + n) {
            dst[d..d + n].copy_from_slice(src);
        }
    }
}

impl Wl {
    /// show the dirty rectangles of window w  [wl_plat_win_present]
    pub(crate) fn win_present(&mut self, w: i32, f: &Frame, rects: Option<&[[i32; 4]]>) {
        let Some(i) = self.rec(w) else { return };
        let wp = &mut self.wp[i];
        let (sw, sh) = (f.w, f.h);
        let fresh = sw != wp.sf_w || sh != wp.sf_h || wp.sf.len() != (sw.max(0) as usize) * (sh.max(0) as usize) * 4;
        if fresh {
            wp.sf = vec![0; (sw.max(0) as usize) * (sh.max(0) as usize) * 4];
            wp.sf_w = sw;
            wp.sf_h = sh;
            if sw > 0 && sh > 0 {
                wp.keep(f, [0, 0, sw, sh]);
            }
        }
        wp.sfok = true;
        match rects {
            None => {
                if !fresh && sw > 0 && sh > 0 {
                    wp.keep(f, [0, 0, sw, sh]);
                }
                wp.pr_add([0, 0, sw, sh]);
            }
            Some(rs) => {
                for r in rs {
                    // clip x, y, x + w, y + h to the frame
                    let b = [r[0].max(0), r[1].max(0), r[0].wrapping_add(r[2]).min(sw), r[1].wrapping_add(r[3]).min(sh)];
                    if b[0] >= b[2] || b[1] >= b[3] {
                        continue;
                    }
                    if !fresh {
                        wp.keep(f, b);
                    }
                    wp.pr_add(b);
                }
            }
        }
        if wp.bx[0] >= wp.bx[2] {
            return; // nothing to show
        }
        wp.pend = true;
        self.present_maybe(i, crate::time_us());
    }

    /// present the window's pending damage now unless it waits for its first configure or
    /// for a frame callback  [present_maybe]
    fn present_maybe(&mut self, i: usize, now: u64) {
        if self.rec(i as i32).is_none() || !self.win[i].shown || !self.win[i].configured {
            return; // not configured yet: later
        }
        let wp = &mut self.wp[i];
        if !wp.pend {
            return;
        }
        if wp.frame_wait {
            if now.wrapping_sub(wp.frame_t0) < FRAME_SAFETY {
                return; // presented when the frame callback fires
            }
            wp.frame_wait = false;
        }
        self.present_do(i, now);
    }

    /// deferred frames -> the next deadline (u64::MAX none)  [wl_present_service]
    pub(crate) fn present_service(&mut self, now: u64) -> u64 {
        let mut next = u64::MAX;
        for i in 0..PW_MAX {
            if self.rec(i as i32).is_none() || !self.win[i].shown || !self.win[i].configured {
                continue;
            }
            let mapped = self.win[i].mapped;
            let wp = &mut self.wp[i];
            // mapped again (or first map without a present yet): the last frame
            if !mapped && wp.sfok && !wp.pend {
                let b = [0, 0, wp.sf_w, wp.sf_h];
                wp.pr_add(b);
                wp.pend = true;
            }
            if !wp.pend {
                continue;
            }
            self.present_maybe(i, now);
            let wp = &self.wp[i];
            if wp.pend && wp.frame_wait {
                next = next.min(wp.frame_t0 + FRAME_SAFETY); // wake up at the safety deadline
            }
        }
        next
    }

    /// the window's surface was destroyed: its frame callback will not come and its buffers
    /// are free (the next present copies everything)  [wl_present_unmapped]
    pub(crate) fn present_unmapped(&mut self, w: usize) {
        let wp = &mut self.wp[w & (PW_MAX - 1)];
        for s in wp.slots.iter_mut() {
            s.busy = false;
            s.st = [0, 0, s.w, s.h];
        }
        wp.frame_cb = 0;
        wp.frame_wait = false;
        wp.sent_gw = 0; // geometry and scale are sent again
        wp.sent_gh = 0;
        wp.sent_scale = 0;
        wp.sent_vw = 0;
        wp.sent_vh = 0;
    }

    /// the window is destroyed: its buffers, pools and mappings go  [wl_present_free]
    pub(crate) fn present_free(&mut self, w: usize) {
        let i = w & (PW_MAX - 1);
        for k in 0..SLOT_N {
            let s = self.wp[i].slots[k];
            if s.buf != 0 {
                self.wp[i].slots[k].buf = 0;
                self.buf_drop(i, s.pool, s.buf);
            }
        }
        for k in 0..ORPH_N {
            let (b, p) = self.wp[i].orph[k];
            if b != 0 {
                self.wp[i].orph[k] = (0, 0);
                self.buf_drop(i, p, b);
            }
        }
        for k in 0..POOL_N {
            let p = self.wp[i].pools[k];
            if !p.used {
                continue;
            }
            if p.id != 0 {
                self.w.send(p.id, &SHM_POOL_DESTROY, &[]);
            }
            // SAFETY: the pool's mapping (made by shm_pool_new, still mapped while used).
            unsafe { syscall6(SYS_MUNMAP, p.map, p.len, 0, 0, 0, 0) };
            self.wp[i].pools[k].used = false;
        }
        self.wp[i] = Wp::default();
    }

    /// a wl_shm_pool of `bytes`: memfd + mmap -> (pool id, mapping address)  [wl_shm_pool_new]
    pub(crate) fn shm_pool_new(&mut self, bytes: usize) -> Option<(u32, usize)> {
        let fail = |bytes: usize| {
            crate::write_stderr(alloc::format!("wayland: cannot create shared memory ({bytes} bytes)\n").as_bytes());
            None
        };
        let name = b"jfgui-shm\0";
        // SAFETY: memfd_create with a NUL-terminated name; MFD_CLOEXEC.
        let fd = unsafe { syscall3(SYS_MEMFD_CREATE, name.as_ptr() as usize, 1, 0) };
        if fd < 0 {
            return fail(bytes);
        }
        let fd = fd as i32;
        loop {
            // SAFETY: our memfd.
            let r = unsafe { syscall3(SYS_FTRUNCATE, fd as usize, bytes, 0) };
            if r == -EINTR {
                continue;
            }
            if r != 0 {
                sys_close(fd);
                return fail(bytes);
            }
            break;
        }
        // SAFETY: a shared read / write mapping of our memfd.
        let map = unsafe { syscall6(SYS_MMAP, 0, bytes, 3, 1, fd as usize, 0) };
        if (-4095..0).contains(&map) {
            sys_close(fd);
            return fail(bytes);
        }
        // the fd is sent, then closed by flush
        let id = self.w.send(self.g.shm, &SHM_CREATE_POOL, &[A::N(&IFACE_WL_SHM_POOL), A::H(fd), A::I(bytes as i32)]);
        if id == 0 {
            sys_close(fd); // no connection
            // SAFETY: the mapping made above.
            unsafe { syscall6(SYS_MUNMAP, map as usize, bytes, 0, 0, 0, 0) };
            return None;
        }
        Some((id, map as usize))
    }

    // ------------------------------------------------------------------ internals

    /// unmap a retired pool without buffers  [pool_check]
    fn pool_check(&mut self, i: usize, pool1: u32) {
        let Some(p) = (pool1 as usize).checked_sub(1).filter(|&p| p < POOL_N) else { return };
        let pl = self.wp[i].pools[p];
        if !pl.used || pl.id != 0 || pl.nbuf != 0 {
            return;
        }
        self.wp[i].pools[p].used = false;
        // SAFETY: the retired pool's mapping; no buffer of it is left.
        unsafe { syscall6(SYS_MUNMAP, pl.map, pl.len, 0, 0, 0, 0) };
    }

    /// destroy a buffer, release its pool reference  [buf_drop]
    fn buf_drop(&mut self, i: usize, pool1: u32, buf: u32) {
        self.w.send(buf, &BUFFER_DESTROY, &[]);
        let p = pool1.wrapping_sub(1) as usize;
        if p >= POOL_N {
            return;
        }
        self.wp[i].pools[p].nbuf = self.wp[i].pools[p].nbuf.wrapping_sub(1);
        self.pool_check(i, pool1);
    }

    /// a new current pool for slots of `need` bytes, retire the old -> ok  [pool_grow]
    fn pool_grow(&mut self, i: usize, need: u64, secondary: bool) -> bool {
        let mut sb = need + (need >> 2); // 1.25 x need
        let min = if secondary { SLOT_MIN_SECONDARY } else { self.max_mode_bytes() as u64 };
        if min > sb {
            sb = min;
        }
        sb = (sb + 4095) & !4095;
        let Some(k) = self.wp[i].pools.iter().position(|p| !p.used) else { return false };
        let len = (sb * SLOT_N as u64) as usize;
        self.wp[i].pools[k].len = len;
        let Some((id, map)) = self.shm_pool_new(len) else { return false };
        self.wp[i].pools[k] = Pool { map, len, id, nbuf: 0, slot: sb as u32, used: true };
        // retire the current pool
        let old = self.wp[i].cur_pool;
        self.wp[i].cur_pool = k as u32 + 1;
        if old == 0 {
            return true;
        }
        let oid = self.wp[i].pools[old as usize - 1].id;
        self.wp[i].pools[old as usize - 1].id = 0;
        self.w.send(oid, &SHM_POOL_DESTROY, &[]);
        for s in 0..SLOT_N {
            let sl = self.wp[i].slots[s];
            if sl.buf == 0 || sl.pool != old {
                continue;
            }
            self.wp[i].slots[s].buf = 0;
            if !sl.busy {
                self.buf_drop(i, old, sl.buf);
                continue;
            }
            self.wp[i].slots[s].busy = false;
            match self.wp[i].orph.iter().position(|o| o.0 == 0) {
                Some(o) => self.wp[i].orph[o] = (sl.buf, old),
                None => self.buf_drop(i, old, sl.buf), // no room: destroy it right away
            }
        }
        self.pool_check(i, old);
        true
    }

    /// copy window i's pending damage into a free slot and commit it  [present_do]
    fn present_do(&mut self, i: usize, now: u64) {
        let r = self.win[i];
        let (w, h) = (self.wp[i].sf_w, self.wp[i].sf_h);
        if !self.wp[i].sfok || w <= 0 || h <= 0 {
            self.present_drop(i);
            return;
        }
        let mut s = (r.s120 as u32 / 120) as i32;
        if s == 0 {
            s = 1;
        }
        if !r.frac && ((w as u32) % s as u32 != 0 || (h as u32) % s as u32 != 0) {
            return; // integer scale: the size must be a multiple
        }
        // a free slot, preferably one that already has this size
        let cur = self.wp[i].cur_pool;
        let mut pick: Option<usize> = None;
        for (k, sl) in self.wp[i].slots.iter().enumerate() {
            if sl.busy {
                continue;
            }
            if sl.buf != 0 && sl.w == w && sl.h == h && sl.pool == cur {
                pick = Some(k);
                break;
            }
            if pick.is_none() {
                pick = Some(k);
            }
        }
        let Some(k) = pick else { return }; // all busy: retry on buffer release
        // pool large enough?
        let need = (w as u32).wrapping_mul(h as u32) as u64 * 4;
        let cur = self.wp[i].cur_pool;
        if (cur == 0 || need > self.wp[i].pools[cur as usize - 1].slot as u64) && !self.pool_grow(i, need, i != 0) {
            return;
        }
        let cur = self.wp[i].cur_pool;
        let sl = self.wp[i].slots[k];
        if sl.buf == 0 || sl.w != w || sl.h != h || sl.pool != cur {
            if sl.buf != 0 {
                self.wp[i].slots[k].buf = 0;
                self.buf_drop(i, sl.pool, sl.buf);
            }
            let pl = self.wp[i].pools[cur as usize - 1];
            let off = pl.slot.wrapping_mul(k as u32);
            let buf = self.w.send(pl.id, &SHM_POOL_CREATE_BUFFER,
                                  &[A::N(&IFACE_WL_BUFFER), A::I(off as i32), A::I(w), A::I(h), A::I(w * 4),
                                    A::U(SHM_XRGB8888)]);
            self.w.set_udata(buf, i as u64); // its window
            self.wp[i].pools[cur as usize - 1].nbuf += 1;
            let slr = &mut self.wp[i].slots[k];
            slr.pool = cur;
            slr.mem = pl.map + off as usize;
            slr.buf = buf;
            slr.w = w;
            slr.h = h;
            slr.st = [0, 0, w, h]; // everything is stale
        }
        // pending rectangles (or their bounding box), then the slot's stale box
        {
            let wp = &mut self.wp[i];
            let sl = wp.slots[k];
            if wp.over {
                copy_box(&sl, &wp.bx, &wp.sf, wp.sf_w);
            } else {
                for b in &wp.pr[..wp.prn] {
                    copy_box(&sl, b, &wp.sf, wp.sf_w);
                }
            }
            if sl.st[0] < sl.st[2] {
                copy_box(&sl, &sl.st, &wp.sf, wp.sf_w);
            }
            wp.slots[k].st[0] = 0;
            wp.slots[k].st[2] = 0;
            // the other slots now miss this damage
            let bx = wp.bx;
            for (o, os) in wp.slots.iter_mut().enumerate() {
                if o != k && os.buf != 0 {
                    box_union(&mut os.st, &bx);
                }
            }
        }
        // configure acknowledgement (only with a buffer of the configured size)
        let matches = w == r.pw && h == r.ph;
        if matches {
            self.win[i].resize_pending = false;
        }
        if self.win[i].ack_pending && (matches || r.flags & WLWF_TOP == 0) {
            // a secondary window's size is ours: its configure is acknowledged anyway
            self.win[i].ack_pending = false;
            self.w.send(r.xsurf, &XDG_SURFACE_ACK_CONFIGURE, &[A::U(r.ack_serial)]);
        }
        // logical size of this buffer
        let (lw, lh) = if !r.frac {
            ((w as u32 / s as u32) as i32, (h as u32 / s as u32) as i32)
        } else if matches {
            (r.lw, r.lh)
        } else {
            let c = r.s120 as u32;
            (((w as u32).wrapping_mul(120).wrapping_add(c >> 1) / c) as i32,
             ((h as u32).wrapping_mul(120).wrapping_add(c >> 1) / c) as i32)
        };
        if lw != self.wp[i].sent_gw || lh != self.wp[i].sent_gh {
            self.wp[i].sent_gw = lw;
            self.wp[i].sent_gh = lh;
            self.w.send(r.xsurf, &XDG_SURFACE_SET_WINDOW_GEOMETRY, &[A::I(0), A::I(0), A::I(lw), A::I(lh)]);
        }
        let bs = if r.frac { 1 } else { s };
        if bs != self.wp[i].sent_scale {
            self.wp[i].sent_scale = bs;
            self.w.send(r.surf, &SURFACE_SET_BUFFER_SCALE, &[A::I(bs)]);
        }
        if r.frac && r.vp != 0 && (lw != self.wp[i].sent_vw || lh != self.wp[i].sent_vh) {
            self.wp[i].sent_vw = lw;
            self.wp[i].sent_vh = lh;
            self.w.send(r.vp, &VIEWPORT_SET_DESTINATION, &[A::I(lw), A::I(lh)]);
        }
        let buf = self.wp[i].slots[k].buf;
        self.w.send(r.surf, &SURFACE_ATTACH, &[A::O(buf), A::I(0), A::I(0)]);
        // damage
        let boxes: Vec<[i32; 4]> = if self.wp[i].over { vec![self.wp[i].bx] } else { self.wp[i].pr[..self.wp[i].prn].to_vec() };
        for b in boxes {
            self.w.send(r.surf, &SURFACE_DAMAGE_BUFFER, &[A::I(b[0]), A::I(b[1]), A::I(b[2] - b[0]), A::I(b[3] - b[1])]);
        }
        // at most one frame callback in flight: while the window is hidden the compositor may
        // never answer, and we must not pile up callback objects
        if self.wp[i].frame_cb == 0 {
            let cb = self.w.send(r.surf, &SURFACE_FRAME, &[A::N(&IFACE_CB_FRAME)]);
            self.wp[i].frame_cb = cb;
            self.w.set_udata(cb, i as u64);
        }
        self.wp[i].frame_wait = true;
        self.wp[i].frame_t0 = now;
        self.w.send(r.surf, &SURFACE_COMMIT, &[]);
        self.w.flush();
        self.wp[i].slots[k].busy = true;
        self.win[i].mapped = true;
        self.present_drop(i);
    }

    /// the pending damage is done (presented or dropped)
    fn present_drop(&mut self, i: usize) {
        let wp = &mut self.wp[i];
        wp.pend = false;
        wp.prn = 0;
        wp.over = false;
        wp.bx[0] = 0;
        wp.bx[2] = 0;
    }
}

// ------------------------------------------------------------------ handlers

/// wl_callback.done of a frame callback  [wlh_frame_done]
pub(crate) fn wlh_frame_done(wl: &mut Wl, id: u32, _a: &Args, u: u64) {
    let wp = &mut wl.wp[u as usize & (PW_MAX - 1)];
    if id == wp.frame_cb {
        wp.frame_cb = 0;
        wp.frame_wait = false;
    }
}

/// the compositor no longer reads this buffer  [wlh_buffer_release]
pub(crate) fn wlh_buffer_release(wl: &mut Wl, id: u32, _a: &Args, u: u64) {
    let i = u as usize & (PW_MAX - 1);
    if let Some(s) = wl.wp[i].slots.iter_mut().find(|s| s.buf == id) {
        s.busy = false;
        return;
    }
    if let Some(k) = wl.wp[i].orph.iter().position(|o| o.0 == id) {
        let p = wl.wp[i].orph[k].1;
        wl.wp[i].orph[k].0 = 0;
        wl.buf_drop(i, p, id);
    }
}
