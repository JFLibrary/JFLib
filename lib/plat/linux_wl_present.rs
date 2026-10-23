// linux_wl_present -- shm buffers and frame presentation (Wayland), per window
//                                                                [src/plat/linux/wl_present.s]
//
//   Wl::win_present(w, frame, rects)   rects = x, y, w, h; None = all.  The frame's pixels
//        (B, G, R, x rows; XRGB8888 == our BGRX memory, no swizzle) go straight into one of
//        three wl_shm buffers of the window: a free buffer gets what it lacks of the frame
//        (its stale boxes: the damage of the frames committed from the other buffers since it
//        was last brought up to date) and waits for its commit; later frames add their damage
//        to it.  It is attached + damaged + committed at once, or -- while a frame callback is
//        outstanding -- when wl_callback.done arrives (or after 1 s, e.g. while minimized;
//        only one callback is ever outstanding per window).  Before the window's first
//        configure (a window just shown) the commit waits for it.  Only with all three
//        buffers busy does the damage go into a staging image; the buffer that frees up then
//        gets its stale boxes from the buffer of the last commit and the staged ones from
//        there.  Pixels outside the rectangles must be what the last present of the window
//        showed.
//   Wl::present_service(now) -> next deadline (u64::MAX none)      (wait)
//   Wl::present_unmapped(w)            the window was unmapped
//   Wl::present_free(w)                the window is destroyed: its buffers and pools
//   Wl::shm_pool_new(bytes) -> (wl_shm_pool, mapping)   memfd + mmap
//   wlh_buffer_release / wlh_frame_done    event handlers (udata = window)
//
// Growing beyond the pool creates a new pool (slot = max(1.25 x need, largest output mode;
// 256 KB for secondary windows)); buffers of the old pool that are still busy are destroyed on
// release and the old mapping is unmapped when none remain.  A window mapped again (shown
// after a hide) gets its last frame when it is configured (from the buffer of the last commit).

use alloc::vec;
use alloc::vec::Vec;

use rt::os::{syscall3, syscall6, SYS_FTRUNCATE, SYS_MEMFD_CREATE, SYS_MMAP, SYS_MUNMAP};

use super::proto::*;
use super::wire::{sys_close, Args, A, EINTR};
use super::window::WLWF_TOP;
use super::Wl;
use crate::linux_damage::{clip_box, copy_boxes, Reg};
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
    mem: usize, // pixels in our mapping
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
    sf: Vec<u8>, // the staging image: damage waiting for a free buffer (rows of sf_w * 4 bytes)
    stg: Reg,    // where it holds frame pixels no buffer has yet
    sf_w: i32,   // the frame size (sfok: a frame was presented)
    sf_h: i32,
    sfok: bool,
    pend: bool, // damage waits to be presented
    prn: usize,
    over: bool,
    bx: [i32; 4], // bounding box (x0, y0, x1, y1)
    pr: [[i32; 4]; PR_MAX],
    back: Option<usize>,   // the buffer with the latest frame that waits for its commit
    newest: Option<usize>, // the buffer of the last commit
    st: [Reg; SLOT_N],     // per buffer: where it differs from the latest frame
    frame_cb: u32,         // outstanding wl_callback (0 = none)
    frame_wait: bool,      // presents wait for it (or FRAME_SAFETY)
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

    /// the buffer of the last commit while it holds a frame of the window's size in the
    /// current pool (its mapping is alive)
    fn newest_ok(&self) -> Option<usize> {
        let n = self.newest?;
        let s = self.slots[n];
        (s.buf != 0 && s.w == self.sf_w && s.h == self.sf_h && s.pool == self.cur_pool).then_some(n)
    }
}

impl Wl {
    /// show the dirty rectangles of window w  [wl_plat_win_present]
    pub(crate) fn win_present(&mut self, w: i32, f: &Frame, rects: Option<&[[i32; 4]]>) {
        let Some(i) = self.rec(w) else { return };
        let wp = &mut self.wp[i];
        let (sw, sh) = (f.w.max(0), f.h.max(0));
        // a new size: everything (the buffers and the pending boxes are of the old one)
        let fresh = !wp.sfok || sw != wp.sf_w || sh != wp.sf_h;
        if fresh {
            wp.sf_w = sw;
            wp.sf_h = sh;
            wp.sf = Vec::new();
            wp.stg.clear();
            wp.back = None;
            wp.newest = None;
            wp.prn = 0;
            wp.over = false;
            wp.bx = [0; 4];
            for st in wp.st.iter_mut() {
                st.set_full(sw, sh);
            }
        }
        wp.sfok = true;
        let all = [[0, 0, sw, sh]];
        let rs = match rects {
            Some(r) if !fresh => r,
            _ => &all[..],
        };
        for r in rs {
            if let Some(b) = clip_box(r, sw, sh) {
                wp.pr_add(b);
                for st in wp.st.iter_mut() {
                    st.add(b); // every buffer now differs there
                }
            }
        }
        if wp.bx[0] >= wp.bx[2] {
            return; // nothing to show
        }
        wp.pend = true;
        self.back_fill(i, f, rs);
        self.present_maybe(i, crate::time_us());
    }

    /// back_fill(i, frame, rects): the frame into the buffer that waits for its commit -- a
    /// free buffer becomes it and gets all it lacks; with every buffer busy the rectangles go
    /// into the staging image
    fn back_fill(&mut self, i: usize, f: &Frame, rs: &[[i32; 4]]) {
        let (w, h) = (self.wp[i].sf_w, self.wp[i].sf_h);
        let (row, ss) = (w as usize * 4, f.stride.max(0) as usize);
        let k = match self.wp[i].back {
            Some(k) => Some(k),
            None => self.slot_pick(i),
        };
        let wp = &mut self.wp[i];
        let Some(k) = k else {
            if wp.sf.len() != row * h as usize {
                wp.sf = vec![0; row * h as usize];
            }
            for r in rs {
                if let Some(b) = clip_box(r, w, h) {
                    copy_boxes(&mut wp.sf, row, f.px, ss, &[b], w, h);
                    wp.stg.add(b);
                }
            }
            if wp.stg.over() {
                let b = wp.stg.bbox(); // all of the bounding box: the region is only that now
                copy_boxes(&mut wp.sf, row, f.px, ss, &[b], w, h);
            }
            return;
        };
        let sl = wp.slots[k];
        // SAFETY: the buffer lies in our mapping, sl.w * sl.h (= w * h) pixels from sl.mem (the
        // pool's slot size is at least that); it is not attached: the compositor does not read it.
        let dst = unsafe { core::slice::from_raw_parts_mut(sl.mem as *mut u8, row * h as usize) };
        copy_boxes(dst, row, f.px, ss, wp.st[k].boxes(), w, h);
        wp.st[k].clear();
        wp.stg.clear(); // (staged boxes are stale boxes of every buffer)
        wp.back = Some(k);
    }

    /// a free buffer of window i of the frame's size (made or remade here: then it lacks
    /// everything), the one of the last commit first -> its slot; None: all busy (or no pool)
    fn slot_pick(&mut self, i: usize) -> Option<usize> {
        let (w, h) = (self.wp[i].sf_w, self.wp[i].sf_h);
        if w <= 0 || h <= 0 {
            return None;
        }
        let wp = &self.wp[i];
        let cur = wp.cur_pool;
        let fits = |k: usize| {
            let s = wp.slots[k];
            !s.busy && s.buf != 0 && s.w == w && s.h == h && s.pool == cur
        };
        let k = wp
            .newest
            .filter(|&n| fits(n))
            .or_else(|| (0..SLOT_N).find(|&k| fits(k)))
            .or_else(|| (0..SLOT_N).find(|&k| !wp.slots[k].busy))?;
        // pool large enough?
        let need = (w as u32).wrapping_mul(h as u32) as u64 * 4;
        if (cur == 0 || need > self.wp[i].pools[cur as usize - 1].slot as u64) && !self.pool_grow(i, need, i != 0) {
            return None;
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
            self.wp[i].st[k].set_full(w, h); // everything is stale
        }
        Some(k)
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
            // mapped again (or first map without a present yet): the whole last frame
            if !mapped && wp.sfok {
                let b = [0, 0, wp.sf_w, wp.sf_h];
                wp.pr_add(b);
                wp.pend = true;
            }
            if !wp.pend {
                continue;
            }
            let t0 = crate::time_us();
            self.present_maybe(i, now);
            crate::present_account(t0, false);
            let wp = &self.wp[i];
            if wp.pend && wp.frame_wait {
                next = next.min(wp.frame_t0 + FRAME_SAFETY); // wake up at the safety deadline
            }
        }
        next
    }

    /// the window's surface was destroyed: its frame callback will not come and its buffers
    /// are free (they keep their pixels)  [wl_present_unmapped]
    pub(crate) fn present_unmapped(&mut self, w: usize) {
        let wp = &mut self.wp[w & (PW_MAX - 1)];
        for s in wp.slots.iter_mut() {
            s.busy = false;
        }
        wp.frame_cb = 0;
        wp.frame_wait = false;
        wp.sent_gw = 0; // geometry and scale are sent again
        wp.sent_gh = 0;
        wp.sent_scale = 0;
        wp.sent_vw = 0;
        wp.sent_vh = 0;
    }

    /// (test aid) window w's last committed buffer: (w, h, rows of w * 4 bytes); None while
    /// damage waits or before the first commit
    pub(crate) fn test_presented(&mut self, w: i32) -> Option<(i32, i32, Vec<u8>)> {
        let i = self.rec(w)?;
        let wp = &self.wp[i];
        if wp.pend {
            return None;
        }
        let s = wp.slots[wp.newest_ok()?];
        // SAFETY: the buffer lies in our live mapping (newest_ok), s.w * s.h pixels.
        let px = unsafe { core::slice::from_raw_parts(s.mem as *const u8, s.w as usize * s.h as usize * 4) };
        Some((s.w, s.h, px.to_vec()))
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
        if self.wp[i].back.is_some_and(|b| self.wp[i].slots[b].pool == old) {
            self.wp[i].back = None;
        }
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

    /// commit the buffer with window i's latest frame; without one waiting, a buffer that
    /// frees up gets what it lacks from the buffer of the last commit and the staging image
    /// [present_do]
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
        let k = match self.wp[i].back {
            Some(k) => k,
            None => {
                let Some(k) = self.slot_pick(i) else { return }; // all busy: retry on buffer release
                let wp = &mut self.wp[i];
                let row = w as usize * 4;
                // SAFETY: as in back_fill: w * h pixels of our mapping, not attached.
                let dst = unsafe { core::slice::from_raw_parts_mut(wp.slots[k].mem as *mut u8, row * h as usize) };
                if let Some(n) = wp.newest_ok().filter(|&n| n != k) {
                    // SAFETY: another buffer of the same (live) pool mapping, w * h pixels, no
                    // overlap with k; the compositor may read it, nobody writes it.
                    let src = unsafe { core::slice::from_raw_parts(wp.slots[n].mem as *const u8, row * h as usize) };
                    for b in wp.st[k].boxes().iter().filter(|b| !wp.stg.covers(b)) {
                        copy_boxes(dst, row, src, row, core::slice::from_ref(b), w, h);
                    }
                }
                copy_boxes(dst, row, &wp.sf, row, wp.stg.boxes(), w, h);
                wp.st[k].clear();
                wp.stg.clear();
                k
            }
        };
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
        self.wp[i].newest = Some(k);
        self.wp[i].back = None;
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
