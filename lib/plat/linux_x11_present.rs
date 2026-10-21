// linux_x11_present -- frame presentation (X11): MIT-SHM or PutImage, per window
//                                                              [src/plat/linux/x11_present.s]
//
//   win_present(w, frame, rects)     rects = x, y, w, h; None = all.  The frame is FMT_XRGB
//        (== ZPixmap depth 24 / 32 bpp LSBFirst, no swizzle).  Its damaged rectangles are
//        copied into the window's shadow image; with MIT-SHM they go into one of the
//        window's two shared segments and are shown with ShmPutImage; the segment is busy
//        until the ShmCompletion of its last put (then the other one is used; with both busy
//        the damage waits, 1 s safety).  Without MIT-SHM (remote, refused attach, any MIT-SHM
//        error) the rectangles go out as PutImage strips that fit the maximum request length.
//   present_service(now) -> next deadline (u64::MAX none)    (wait)
//   expose(ev)                       Expose: repaint from the shadow image
//   present_mapped(i)                MapNotify: the whole window is shown again 250 ms and
//                                    1 s later (WSLg: the first frames of a new RAIL window
//                                    are sometimes lost, the window stays black until the
//                                    next damage)
//   shm_done(ev)                     ShmCompletion: the segment is free again
//   present_error(minor, code)       an MIT-SHM request failed: use PutImage
//   present_free(i)                  the window is destroyed: its segments go
//
// Segments: memfd + ShmAttachFd (MIT-SHM 1.2, works across IPC namespaces) or SysV
// shmget/shmat + ShmAttach (then IPC_RMID).  jfgui_X11SHM=off|sysv|fd forces a mode.
// Segment size = max(1.25 x the frame, the screen) for the main window, max(1.25 x the
// frame, 256 KB) for the others, so resizing rarely reallocates.

use alloc::format;
use alloc::vec;
use alloc::vec::Vec;

use rt::os::{syscall3, syscall6};

use super::wire::sys_close;
use super::*;

const REPAINT_FIRST: u64 = 250000; // us after MapNotify
const REPAINT_AGAIN: u64 = 750000; // us after the first one
const SHM_SAFETY: u64 = 1000000; // us to wait for a ShmCompletion
const SEG_MIN_SECONDARY: usize = 262144; // smallest segment of a secondary window
const MODE_OFF: i32 = -1;
const MODE_FD: i32 = 1;
const MODE_SYSV: i32 = 2;
const ZPIXMAP: u32 = 2;
const IPC_PRIVATE: usize = 0;
const IPC_CREAT: usize = 0x200;
const IPC_RMID: usize = 0;
const PROT_RW: usize = 3;
const MAP_SHARED: usize = 1;
const MFD_CLOEXEC: usize = 1;

impl Xp {
    /// pr_add(x0, y0, x1, y1): add a (clipped, non-empty) box to the pending damage
    fn add(&mut self, x0: i32, y0: i32, x1: i32, y1: i32) {
        self.pend = true;
        let b = &mut self.bx;
        if b[0] >= b[2] {
            *b = [x0, y0, x1, y1];
        } else {
            b[0] = b[0].min(x0);
            b[1] = b[1].min(y0);
            b[2] = b[2].max(x1);
            b[3] = b[3].max(y1);
        }
        if self.pr.len() < PR_MAX {
            self.pr.push([x0, y0, x1, y1]);
        } else {
            self.over = true;
        }
    }

    /// pr_add_clip(x0, y0, x1, y1): clip to the shadow image, then add
    fn add_clip(&mut self, x0: i32, y0: i32, x1: i32, y1: i32) {
        let (x0, y0, x1, y1) = (x0.max(0), y0.max(0), x1.min(self.sw), y1.min(self.shh));
        if x0 < x1 && y0 < y1 {
            self.add(x0, y0, x1, y1);
        }
    }

    /// rect_list: the pending list, or only its bounding box when it overflowed -> count
    fn nrects(&self) -> usize {
        if self.over { 1 } else { self.pr.len() }
    }

    fn rect(&self, j: usize) -> [i32; 4] {
        if self.over { self.bx } else { self.pr[j] }
    }

    fn drop_pending(&mut self) {
        self.pend = false;
        self.pr.clear();
        self.over = false;
        self.bx[0] = 0;
        self.bx[2] = 0;
    }
}

impl X11 {
    /// x11_plat_win_present(w, frame, rects)
    pub(super) fn win_present(&mut self, w: i32, f: &Frame, rects: Option<&[[i32; 4]]>) {
        let Some(i) = self.rec(w) else { return };
        let xp = &mut self.xp[i];
        let (fw, fh) = (f.w.max(0), f.h.max(0));
        let row = fw as usize * 4;
        // the shadow image: everything when the size changed, else the damaged rectangles
        let full = !xp.sfok || xp.sw != fw || xp.shh != fh;
        if full {
            xp.sh = vec![0u8; row * fh as usize];
            xp.sw = fw;
            xp.shh = fh;
            for y in 0..fh as usize {
                let s = y * f.stride as usize;
                xp.sh[y * row..y * row + row].copy_from_slice(&f.px[s..s + row]);
            }
        }
        xp.sfok = true;
        match rects {
            None => xp.add(0, 0, fw, fh),
            Some(list) => {
                for r in list {
                    let (x0, y0) = (r[0].max(0), r[1].max(0));
                    let (x1, y1) = (r[0].wrapping_add(r[2]).min(fw), r[1].wrapping_add(r[3]).min(fh));
                    if !full && x0 < x1 && y0 < y1 {
                        let n = (x1 - x0) as usize * 4;
                        for y in y0 as usize..y1 as usize {
                            let s = y * f.stride as usize + x0 as usize * 4;
                            let d = y * row + x0 as usize * 4;
                            xp.sh[d..d + n].copy_from_slice(&f.px[s..s + n]);
                        }
                    }
                    xp.add_clip(r[0], r[1], r[0].wrapping_add(r[2]), r[1].wrapping_add(r[3]));
                }
            }
        }
        let now = crate::time_us();
        self.present_try(i, now);
    }

    /// x11_present_mapped(i): the window was mapped -- repaint all of it again later
    pub(super) fn present_mapped(&mut self, i: usize) {
        let xp = &mut self.xp[i & (PW_MAX - 1)];
        xp.rpt = crate::time_us() + REPAINT_FIRST;
        xp.rptn = 2;
    }

    /// x11_present_service(now) -> next deadline or u64::MAX
    pub(super) fn present_service(&mut self, now: u64) -> u64 {
        let mut next = u64::MAX;
        for i in 0..PW_MAX {
            let xp = &mut self.xp[i];
            if xp.rpt != 0 {
                // a full repaint after a map
                if now >= xp.rpt {
                    xp.rpt = 0;
                    xp.rptn -= 1;
                    if xp.rptn > 0 {
                        xp.rpt = now + REPAINT_AGAIN;
                        next = next.min(xp.rpt);
                    }
                    if xp.sfok && xp.sw > 0 && xp.shh > 0 {
                        let (w, h) = (xp.sw, xp.shh);
                        xp.add(0, 0, w, h);
                    }
                } else {
                    next = next.min(xp.rpt);
                }
            }
            if self.xp[i].pend {
                self.present_try(i, now);
                if self.xp[i].pend {
                    // waiting for a segment: wake up at the safety deadline of the older one
                    for s in self.xp[i].slots {
                        if s.busy {
                            next = next.min(s.t0 + SHM_SAFETY);
                        }
                    }
                }
            }
        }
        next
    }

    /// x11_expose(ev): Expose(window, x, y, w, h, count) -- show that area again
    pub(super) fn expose(&mut self, b: &[u8]) {
        let i = self.find(rd32(b, 4));
        if i < 0 {
            return;
        }
        let xp = &mut self.xp[i as usize];
        if !xp.sfok {
            return;
        }
        let (x, y) = (rd16(b, 8) as i32, rd16(b, 10) as i32);
        xp.add_clip(x, y, x + rd16(b, 12) as i32, y + rd16(b, 14) as i32);
    }

    /// x11_shm_done(ev): ShmCompletion(drawable, minor, major, shmseg, offset) -- that segment
    /// (of whichever window) is free again
    pub(super) fn shm_done(&mut self, b: &[u8]) {
        let seg = rd32(b, 12);
        if seg == 0 {
            return;
        }
        for xp in self.xp.iter_mut() {
            for s in xp.slots.iter_mut() {
                if s.seg == seg {
                    s.busy = false;
                }
            }
        }
    }

    /// x11_present_error(minor, code): an MIT-SHM request failed -- stop using shared memory
    /// and repaint everything with PutImage (errors of an attach being tested are handled by
    /// slot_alloc)
    pub(super) fn present_error(&mut self, minor: u32, code: u32) {
        if self.attaching || self.mode == MODE_OFF {
            return;
        }
        self.mode = MODE_OFF;
        if self.debug {
            eprint(&format!("x11: MIT-SHM request {minor} failed (error {code}): using PutImage\n"));
        }
        for xp in self.xp.iter_mut() {
            xp.slots[0].busy = false;
            xp.slots[1].busy = false;
            if xp.sfok {
                let (w, h) = (xp.sw, xp.shh);
                xp.add(0, 0, w, h);
            }
        }
    }

    /// x11_present_free(i): the window is destroyed -- detach and unmap its segments
    pub(super) fn present_free(&mut self, i: usize) {
        let i = i & (PW_MAX - 1);
        self.slot_free(i, 0);
        self.slot_free(i, 1);
        let xp = &mut self.xp[i];
        xp.sh = Vec::new();
        xp.sw = 0;
        xp.shh = 0;
        xp.sfok = false;
        xp.drop_pending();
        xp.slots = [SEG0; 2];
        xp.rpt = 0;
        xp.rptn = 0;
    }

    // ------------------------------------------------------------------ presenting

    /// present_try(i, now): show the pending damage of window i (unless both segments are busy)
    fn present_try(&mut self, i: usize, now: u64) {
        let Some(_) = self.rec(i as i32) else { return };
        let xid = self.xw[i].win;
        if !self.xp[i].pend {
            return;
        }
        let (w, h) = (self.xp[i].sw, self.xp[i].shh);
        if !self.xp[i].sfok || w <= 0 || h <= 0 || w > 32767 || h > 32767 {
            self.xp[i].drop_pending();
            return;
        }
        if self.mode == 0 {
            self.shm_decide();
        }
        let mut put = self.mode <= 0;
        if !put {
            // a free segment (or one whose completion is overdue)
            let mut k = None;
            for s in 0..2 {
                let sl = self.xp[i].slots[s];
                if !sl.busy {
                    k = Some(s);
                    break;
                }
                if sl.t0 + SHM_SAFETY < now {
                    if self.debug {
                        eprint(&format!("x11: no ShmCompletion for segment {:x} within 1 s\n", sl.seg));
                    }
                    k = Some(s);
                    break;
                }
            }
            let Some(k) = k else { return }; // retried on ShmCompletion
            let need = w as usize * h as usize * 4;
            if need > self.xp[i].slots[k].cap && !self.slot_alloc(i, k, need, i != 0) {
                self.mode = MODE_OFF;
                put = true;
            }
            if !put {
                // copy each rectangle into the segment (stride = width * 4), then put it
                let n = self.xp[i].nrects();
                for j in 0..n {
                    let r = self.xp[i].rect(j);
                    self.copy_rect(i, k, &r);
                    self.shm_put(i, k, &r, j + 1 == n, xid);
                }
                let sl = &mut self.xp[i].slots[k];
                sl.busy = true;
                sl.t0 = now;
            }
        }
        if put {
            for j in 0..self.xp[i].nrects() {
                let r = self.xp[i].rect(j);
                self.put_rect(i, &r, xid);
            }
        }
        self.flush();
        self.xp[i].drop_pending();
    }

    /// copy_rect(slot, box): shadow rows of the box into the segment
    fn copy_rect(&mut self, i: usize, k: usize, r: &[i32; 4]) {
        let xp = &self.xp[i];
        let sl = xp.slots[k];
        let (x0, y0, x1, y1) = (r[0], r[1], r[2], r[3]);
        if x1 <= x0 {
            return;
        }
        let n = (x1 - x0) as usize * 4;
        for y in y0.max(0)..y1 {
            let o = (y as usize * xp.sw as usize + x0 as usize) * 4;
            let src = &xp.sh[o..o + n];
            // SAFETY: o + n <= sw * shh * 4 <= the segment's cap (checked before the copy).
            unsafe { core::ptr::copy_nonoverlapping(src.as_ptr(), sl.mem.add(o), n) };
        }
    }

    /// shm_put(slot, box, send_event, window): ShmPutImage of the box (same place in the window)
    fn shm_put(&mut self, i: usize, k: usize, r: &[i32; 4], send: bool, xid: u32) {
        let (op, gc, seg) = (self.shm_op, self.gc, self.xp[i].slots[k].seg);
        let (tw, th) = (self.xp[i].sw, self.xp[i].shh);
        let (o, _) = self.req(op | XSHM_PUT_IMAGE << 8, 40);
        let b = &mut self.out;
        wr32(b, o, xid);
        wr32(b, o + 4, gc);
        wr16(b, o + 8, tw as u32); // total width (= segment stride / 4)
        wr16(b, o + 10, th as u32);
        wr16(b, o + 12, r[0] as u32); // src x, y
        wr16(b, o + 14, r[1] as u32);
        wr16(b, o + 16, (r[2] - r[0]) as u32); // width
        wr16(b, o + 18, (r[3] - r[1]) as u32); // height
        wr16(b, o + 20, r[0] as u32); // dst x, y
        wr16(b, o + 22, r[1] as u32);
        b[o + 24] = 24; // depth
        b[o + 25] = ZPIXMAP as u8;
        b[o + 26] = send as u8;
        b[o + 27] = 0;
        wr32(b, o + 28, seg);
        wr32(b, o + 32, 0); // offset
    }

    /// put_rect(box, window): PutImage (ZPixmap, depth 24) in strips of whole rows that fit
    /// the maximum request length
    fn put_rect(&mut self, i: usize, r: &[i32; 4], xid: u32) {
        let (x0, mut y, y1) = (r[0], r[1], r[3]);
        let width = r[2] - x0;
        if width <= 0 {
            return;
        }
        let bpr = width as u32 * 4; // bytes per row
        let max = self.maxreq.min(X11_OUT_CAP as u32);
        let rows = ((max - 32) / bpr).max(1);
        let gc = self.gc;
        while y < y1 {
            let hh = ((y1 - y) as u32).min(rows);
            let (o, _) = self.req(XR_PUT_IMAGE | ZPIXMAP << 8, (hh * bpr + 24) as usize);
            let sw = self.xp[i].sw as usize;
            let b = &mut self.out;
            wr32(b, o, xid);
            wr32(b, o + 4, gc);
            wr16(b, o + 8, width as u32);
            wr16(b, o + 10, hh);
            wr16(b, o + 12, x0 as u32);
            wr16(b, o + 14, y as u32);
            b[o + 16] = 0; // left pad
            b[o + 17] = 24; // depth
            wr16(b, o + 18, 0);
            let mut d = o + 20;
            for _ in 0..hh {
                let s = (y as usize * sw + x0 as usize) * 4;
                b[d..d + bpr as usize].copy_from_slice(&self.xp[i].sh[s..s + bpr as usize]);
                d += bpr as usize;
                y += 1;
            }
        }
    }

    // ------------------------------------------------------------------ shared memory

    /// shm_decide: MODE_FD (MIT-SHM >= 1.2), MODE_SYSV, or MODE_OFF; jfgui_X11SHM
    fn shm_decide(&mut self) {
        let mut m = MODE_OFF;
        if self.shm_op != 0 {
            m = if self.shm_minor >= 2 { MODE_FD } else { MODE_SYSV };
            match crate::env("jfgui_X11SHM").as_deref() {
                Some("off") => m = MODE_OFF,
                Some("sysv") => m = MODE_SYSV,
                _ => {}
            }
        }
        self.mode = m;
        if self.debug {
            eprint(&format!("x11: present mode {m} (1 MIT-SHM fd, 2 MIT-SHM SysV, -1 PutImage)\n"));
        }
    }

    /// slot_free(slot): detach and unmap a segment
    fn slot_free(&mut self, i: usize, k: usize) {
        let sl = self.xp[i].slots[k];
        if sl.seg == 0 {
            return;
        }
        if self.mode > 0 {
            let (o, _) = self.req(self.shm_op | XSHM_DETACH << 8, 8);
            wr32(&mut self.out, o, sl.seg);
        }
        let s = &mut self.xp[i].slots[k];
        s.seg = 0;
        s.busy = false;
        s.cap = 0;
        // SAFETY: mem / cap describe our own mapping of this segment.
        unsafe {
            if sl.shmid == -1 {
                syscall3(SYS_MUNMAP, sl.mem as usize, sl.cap, 0);
            } else {
                syscall3(SYS_SHMDT, sl.mem as usize, 0, 0);
            }
        }
    }

    /// slot_alloc(slot, bytes, secondary) -> a new shared segment of at least bytes (1.25 x;
    /// at least the screen, for secondary windows at least 256 KB); the fd mode falls back to
    /// SysV once when the server refuses it
    fn slot_alloc(&mut self, i: usize, k: usize, bytes: usize, secondary: bool) -> bool {
        let mut size = bytes + (bytes >> 2);
        let min = if secondary { SEG_MIN_SECONDARY } else { self.scr_w as usize * self.scr_h as usize * 4 };
        size = size.max(min);
        size = (size + 4095) & !4095;
        self.slot_free(i, k);
        loop {
            if self.mode != MODE_SYSV {
                // ---- memfd + ShmAttachFd
                if let Some((mem, seg)) = self.attach_fd(size) {
                    let s = &mut self.xp[i].slots[k];
                    *s = Seg { seg, busy: false, shmid: -1, cap: size, mem, t0: s.t0 };
                    return true;
                }
                self.mode = MODE_SYSV;
                if self.debug {
                    eprint("x11: ShmAttachFd failed, trying SysV shared memory\n");
                }
                continue;
            }
            // ---- shmget + shmat + ShmAttach, removed after the server attached it
            if let Some((mem, seg, id)) = self.attach_sysv(size) {
                let s = &mut self.xp[i].slots[k];
                *s = Seg { seg, busy: false, shmid: id, cap: size, mem, t0: s.t0 };
                return true;
            }
            if self.debug {
                eprint("x11: no shared memory with the X server: using PutImage\n");
            }
            return false;
        }
    }

    /// memfd + ShmAttachFd -> (mapping, shmseg)
    fn attach_fd(&mut self, size: usize) -> Option<(*mut u8, u32)> {
        let name = b"jfgui-x11\0";
        // SAFETY: memfd_create with a NUL-terminated name.
        let fd = unsafe { syscall3(SYS_MEMFD_CREATE, name.as_ptr() as usize, MFD_CLOEXEC, 0) };
        if fd < 0 {
            return None;
        }
        let fd = fd as i32;
        loop {
            // SAFETY: our new descriptor.
            let r = unsafe { syscall3(SYS_FTRUNCATE, fd as usize, size, 0) };
            if r == -EINTR {
                continue;
            }
            if r != 0 {
                sys_close(fd);
                return None;
            }
            break;
        }
        // SAFETY: a new shared mapping of the memfd.
        let mem = unsafe { syscall6(SYS_MMAP, 0, size, PROT_RW, MAP_SHARED, fd as usize, 0) };
        if (mem as usize) >= (-4095isize) as usize {
            sys_close(fd);
            return None;
        }
        let seg = self.new_id();
        self.attaching = true;
        let (o, seq) = self.req(self.shm_op | XSHM_ATTACH_FD << 8, 12);
        wr32(&mut self.out, o, seg);
        self.out[o + 4] = 1; // read-only
        self.watch(seq);
        self.out_fd(fd); // sent with the request, then closed
        self.sync();
        let err = self.watch_error();
        self.attaching = false;
        if err != 0 {
            // SAFETY: the mapping made above.
            unsafe { syscall3(SYS_MUNMAP, mem as usize, size, 0) };
            return None;
        }
        Some((mem as *mut u8, seg))
    }

    /// shmget + shmat + ShmAttach (then IPC_RMID) -> (mapping, shmseg, id)
    fn attach_sysv(&mut self, size: usize) -> Option<(*mut u8, u32, i32)> {
        // SAFETY: shmget / shmat / shmctl / shmdt on a private segment of our own.
        unsafe {
            let id = syscall3(SYS_SHMGET, IPC_PRIVATE, size, IPC_CREAT | 0x180);
            if id < 0 {
                return None;
            }
            let mem = syscall3(SYS_SHMAT, id as usize, 0, 0);
            if (mem as usize) >= (-4095isize) as usize {
                syscall3(SYS_SHMCTL, id as usize, IPC_RMID, 0);
                return None;
            }
            let seg = self.new_id();
            self.attaching = true;
            let (o, seq) = self.req(self.shm_op | XSHM_ATTACH << 8, 16);
            wr32(&mut self.out, o, seg);
            wr32(&mut self.out, o + 4, id as u32);
            self.out[o + 8] = 1; // read-only
            self.watch(seq);
            self.sync();
            let err = self.watch_error();
            self.attaching = false;
            syscall3(SYS_SHMCTL, id as usize, IPC_RMID, 0); // gone once both sides detach
            if err != 0 {
                syscall3(SYS_SHMDT, mem as usize, 0, 0);
                return None;
            }
            Some((mem as *mut u8, seg, id as i32))
        }
    }
}
