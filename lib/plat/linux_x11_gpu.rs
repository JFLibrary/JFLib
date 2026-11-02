// linux_x11_gpu -- GPU buffers on an X window: DRI3 and Present (programs against glibc;
// linux_gpuout.rs says what the calls mean and why no wrong frame can show)
//
//   gpu_caps(w)      DRI3 1.2 and Present 1.0 or the CPU path; DRI3Open says which device the server
//                    renders with (the reply carries a file descriptor: recv here is recvmsg);
//                    GetSupportedModifiers for depth 24, 32 bits per pixel: XRGB8888
//   gpu_buf_new      DRI3 PixmapFromBuffers (an error: the buffer is refused, the CPU path)
//   gpu_show         PresentPixmap with the rectangles as the update region (XFixes; without it the
//                    whole window).  At most one on its way: the next frame starts after its
//                    PresentCompleteNotify (1 s at most), as on Wayland after the frame callback
//   a pixmap is free again after its PresentIdleNotify
//   Expose           CopyArea from the newest pixmap; the repaints after a map (for WSLg's first
//                    frames, linux_x11_present.rs) are the shared memory path's and do not run here:
//                    they would put an old frame on the window
//   win_present      ends the buffers' turn (WpGpu::leave); the server sorts out a window that was
//                    flipped to a pixmap, drawing on it works by definition

use alloc::vec::Vec;

use rt::os::{syscall3, syscall6};

use super::wire::sys_close;
use super::*;
use crate::{GpuBufDesc, GpuCaps, GPU_BUF_BUSY, GPU_BUF_FAILED, GPU_BUF_FREE};

const SYS_RECVMSG: usize = 47;
const SYS_FSTAT: usize = 5;
const MSG_DONTWAIT: usize = 0x40;
const MSG_CMSG_CLOEXEC: usize = 0x4000_0000;
const SOL_SOCKET: u32 = 1;
const SCM_RIGHTS: u32 = 1;
const DRM_FORMAT_XRGB8888: u32 = 0x3432_5258; // 'XR24'
const SAFETY: u64 = 1_000_000; // us to wait for PresentCompleteNotify
const IDLE_LOOK: u64 = 100_000; // us: an answer that never comes
const DAMAGE_MAX: usize = 16;
const DRI3_QUERY_VERSION: u32 = 0;
const DRI3_OPEN: u32 = 1;
const DRI3_GET_SUPPORTED_MODIFIERS: u32 = 6;
const DRI3_PIXMAP_FROM_BUFFERS: u32 = 7;
const PRESENT_QUERY_VERSION: u32 = 0;
const PRESENT_PIXMAP: u32 = 1;
const PRESENT_SELECT_INPUT: u32 = 3;
const PRESENT_EV_COMPLETE: u16 = 1;
const PRESENT_EV_IDLE: u16 = 2;
const PRESENT_MASK_COMPLETE: u32 = 2;
const PRESENT_MASK_IDLE: u32 = 4;
const XFIXES_QUERY_VERSION: u32 = 0;
const XFIXES_CREATE_REGION: u32 = 5;
const XFIXES_SET_REGION: u32 = 11;
const XR_FREE_PIXMAP: u32 = 54;
const XR_COPY_AREA: u32 = 62;

/// the connection's part
#[derive(Default)]
pub(super) struct XGpu {
    probed: bool,
    dri3_op: u32,
    present_op: u32,
    xfixes_op: u32,
    device: Option<u64>,
    next: u32,
    serial: u32,
    /// a descriptor that came with a reply (recvmsg): DRI3Open's
    in_fd: i32,
    /// the loop in wait() goes back to its caller
    pub wake: bool,
}

struct Buf {
    h: u32,
    pixmap: u32,
    busy: bool,
}

/// a window's buffers (in its presentation record)
#[derive(Default)]
pub(super) struct XpGpu {
    pub active: bool,
    selected: bool,
    region: u32,
    bufs: Vec<Buf>,
    newest: u32,
    pend: u32,
    full: bool,
    damage: Vec<[i32; 4]>,
    wait_complete: bool,
    serial: u32,
    t0: u64,
    want: bool,
}

impl XpGpu {
    /// the CPU path takes over: the frame on its way is dropped
    pub(super) fn leave(&mut self) {
        self.active = false;
        self.pend = 0;
        self.newest = 0;
        self.damage.clear();
        self.full = false;
    }
}

impl X11 {
    pub(super) fn gpu_present_op(&self) -> u32 {
        self.g.present_op
    }

    /// the receive of the wire with the descriptors of a reply kept
    pub(super) fn gpu_recv(&mut self, room: usize) -> isize {
        let iov = [self.inb.as_mut_ptr() as usize + self.in_len, room];
        let mut ctl = [0u64; 6];
        let mut mh = [0usize; 7]; // msghdr: name, namelen, iov, iovlen, control, controllen, flags
        mh[2] = iov.as_ptr() as usize;
        mh[3] = 1;
        mh[4] = ctl.as_mut_ptr() as usize;
        mh[5] = 48;
        // SAFETY: mh, iov and ctl describe live buffers for the call.
        let r = unsafe { syscall3(SYS_RECVMSG, self.fd as usize, mh.as_mut_ptr() as usize, MSG_DONTWAIT | MSG_CMSG_CLOEXEC) };
        if r > 0 && mh[5] >= 20 && ctl[1] == (SOL_SOCKET as u64 | (SCM_RIGHTS as u64) << 32) {
            let n = (ctl[0] as usize).saturating_sub(16) / 4;
            for k in 0..n.min(8) {
                let fd = (ctl[2 + k / 2] >> (32 * (k % 2))) as u32 as i32;
                if self.g.in_fd <= 0 {
                    self.g.in_fd = fd;
                } else {
                    sys_close(fd);
                }
            }
        }
        r
    }

    /// an extension's major opcode (0: none)
    fn gpu_ext(&mut self, name: &[u8]) -> u32 {
        let seq = self.query_ext_send(name);
        self.reply(seq).map_or(0, |r| if r[8] != 0 { r[9] as u32 } else { 0 })
    }

    /// QueryVersion of an extension -> (major, minor)
    fn gpu_version(&mut self, op: u32, major: u32, minor: u32) -> (u32, u32) {
        let (o, seq) = self.req(op, 12);
        wr32(&mut self.out, o, major);
        wr32(&mut self.out, o + 4, minor);
        self.reply(seq).map_or((0, 0), |r| (rd32(&r, 8), rd32(&r, 12)))
    }

    fn gpu_probe(&mut self) {
        self.g.probed = true;
        let (dri3, present, xfixes) = (self.gpu_ext(b"DRI3"), self.gpu_ext(b"Present"), self.gpu_ext(b"XFIXES"));
        if dri3 == 0 || present == 0 {
            return;
        }
        let v = self.gpu_version(dri3 | DRI3_QUERY_VERSION << 8, 1, 2);
        let p = self.gpu_version(present | PRESENT_QUERY_VERSION << 8, 1, 2);
        if v.0 != 1 || v.1 < 2 || p.0 != 1 {
            return;
        }
        if xfixes != 0 && self.gpu_version(xfixes | XFIXES_QUERY_VERSION << 8, 2, 0).0 >= 2 {
            self.g.xfixes_op = xfixes;
        }
        // which device: DRI3Open's descriptor (a render node), its st_rdev
        let root = self.root;
        let (o, seq) = self.req(dri3 | DRI3_OPEN << 8, 12);
        wr32(&mut self.out, o, root);
        self.g.in_fd = -1;
        if self.reply(seq).is_none() {
            return; // no DRI3 on this server's screen
        }
        let fd = core::mem::replace(&mut self.g.in_fd, -1);
        if fd >= 0 {
            let mut st = [0u64; 18];
            // SAFETY: struct stat fits (144 bytes on x86-64); a descriptor of ours.
            if unsafe { syscall6(SYS_FSTAT, fd as usize, st.as_mut_ptr() as usize, 0, 0, 0, 0) } == 0 {
                self.g.device = Some(st[5]); // st_rdev
            }
            sys_close(fd);
        }
        self.g.dri3_op = dri3;
        self.g.present_op = present;
    }

    pub(super) fn gpu_caps(&mut self, w: i32) -> Option<GpuCaps> {
        let i = self.rec(w)?;
        if !self.g.probed {
            self.gpu_probe();
        }
        if self.g.dri3_op == 0 {
            return None;
        }
        let xid = self.xw[i].win;
        let (o, seq) = self.req(self.g.dri3_op | DRI3_GET_SUPPORTED_MODIFIERS << 8, 12);
        wr32(&mut self.out, o, xid);
        self.out[o + 4] = 24; // depth
        self.out[o + 5] = 32; // bits per pixel
        let r = self.reply(seq)?;
        let n = rd32(&r, 8) as usize + rd32(&r, 12) as usize;
        let mut formats: Vec<(u32, u64)> = Vec::new();
        for k in 0..n {
            let at = 32 + k * 8;
            if at + 8 > r.len() {
                break;
            }
            let m = rd32(&r, at) as u64 | (rd32(&r, at + 4) as u64) << 32;
            if !formats.contains(&(DRM_FORMAT_XRGB8888, m)) {
                formats.push((DRM_FORMAT_XRGB8888, m));
            }
        }
        Some(GpuCaps { device: self.g.device, formats })
    }

    pub(super) fn gpu_buf_new(&mut self, w: i32, d: &GpuBufDesc) -> u32 {
        let (Some(i), [p]) = (self.rec(w), &d.planes[..]) else {
            crate::linux::gpuout::close_planes(d);
            return 0;
        };
        if self.g.dri3_op == 0 || d.fourcc != DRM_FORMAT_XRGB8888 || d.w > 32767 || d.h > 32767 {
            sys_close(p.fd);
            return 0;
        }
        let xid = self.xw[i].win;
        let pixmap = self.new_id();
        self.flush(); // (out_fd goes with the next flush: this request alone)
        let (o, seq) = self.req(self.g.dri3_op | DRI3_PIXMAP_FROM_BUFFERS << 8, 64);
        let b = &mut self.out;
        wr32(b, o, pixmap);
        wr32(b, o + 4, xid);
        b[o + 8] = 1; // num_buffers
        wr16(b, o + 12, d.w as u32);
        wr16(b, o + 14, d.h as u32);
        wr32(b, o + 16, p.stride);
        wr32(b, o + 20, p.offset);
        b[o + 48] = 24; // depth
        b[o + 49] = 32; // bits per pixel
        wr32(b, o + 52, d.modifier as u32);
        wr32(b, o + 56, (d.modifier >> 32) as u32);
        self.watch(seq);
        self.out_fd(p.fd); // sent with the request, then closed
        self.sync();
        if self.watch_error() != 0 {
            return 0;
        }
        self.g.next = self.g.next.wrapping_add(1).max(1);
        let h = self.g.next;
        self.xp[i].gpu.bufs.push(Buf { h, pixmap, busy: false });
        h
    }

    pub(super) fn gpu_buf_state(&mut self, w: i32, h: u32) -> u32 {
        let Some(i) = self.rec(w) else { return GPU_BUF_FAILED };
        let g = &mut self.xp[i].gpu;
        let (newest, pend) = (g.newest, g.pend);
        let Some(b) = g.bufs.iter().find(|b| b.h == h) else { return GPU_BUF_FAILED };
        if b.busy || h == newest || h == pend {
            g.want = true;
            return GPU_BUF_BUSY;
        }
        GPU_BUF_FREE
    }

    pub(super) fn gpu_buf_free(&mut self, w: i32, h: u32) {
        let Some(i) = self.rec(w) else { return };
        let g = &mut self.xp[i].gpu;
        let Some(k) = g.bufs.iter().position(|b| b.h == h) else { return };
        let b = g.bufs.swap_remove(k);
        if g.newest == h {
            g.newest = 0;
        }
        if g.pend == h {
            g.pend = 0;
        }
        let (o, _) = self.req(XR_FREE_PIXMAP, 8);
        wr32(&mut self.out, o, b.pixmap);
        self.flush();
    }

    /// the window is destroyed: its pixmaps go (the region and the event selection with the connection)
    pub(super) fn gpu_free_all(&mut self, i: usize) {
        for b in core::mem::take(&mut self.xp[i].gpu.bufs) {
            let (o, _) = self.req(XR_FREE_PIXMAP, 8);
            wr32(&mut self.out, o, b.pixmap);
        }
        self.xp[i].gpu = XpGpu::default();
    }

    pub(super) fn gpu_can_show(&mut self, w: i32) -> bool {
        let Some(i) = self.rec(w) else { return false };
        let now = crate::time_us();
        let g = &mut self.xp[i].gpu;
        if g.pend != 0 || (g.wait_complete && now.wrapping_sub(g.t0) < SAFETY) {
            g.want = true;
            return false;
        }
        true
    }

    pub(super) fn gpu_show(&mut self, w: i32, h: u32, rects: Option<&[[i32; 4]]>) {
        let Some(i) = self.rec(w) else { return };
        if !self.xp[i].gpu.bufs.iter().any(|b| b.h == h) {
            return;
        }
        if !self.xp[i].gpu.active {
            // what the shared memory path still wanted to put is over
            let xp = &mut self.xp[i];
            xp.drop_pending();
            xp.stg.clear();
            xp.gpu.active = true;
            xp.gpu.full = true;
            xp.gpu.damage.clear();
            if !xp.gpu.selected {
                xp.gpu.selected = true;
                self.gpu_select(i);
            }
        }
        let g = &mut self.xp[i].gpu;
        g.pend = h;
        match rects {
            Some(r) if !g.full => {
                g.damage.extend_from_slice(r);
                if g.damage.len() > DAMAGE_MAX {
                    g.full = true;
                }
            }
            _ => g.full = true,
        }
        self.gpu_try(i, crate::time_us());
    }

    fn gpu_select(&mut self, i: usize) {
        let xid = self.xw[i].win;
        let eid = self.new_id();
        let (o, _) = self.req(self.g.present_op | PRESENT_SELECT_INPUT << 8, 16);
        wr32(&mut self.out, o, eid);
        wr32(&mut self.out, o + 4, xid);
        wr32(&mut self.out, o + 8, PRESENT_MASK_COMPLETE | PRESENT_MASK_IDLE);
    }

    /// the waiting buffer goes on the window when the last one has been answered
    fn gpu_try(&mut self, i: usize, now: u64) {
        let g = &mut self.xp[i].gpu;
        if g.pend == 0 {
            return;
        }
        if g.wait_complete {
            if now.wrapping_sub(g.t0) < SAFETY {
                return;
            }
            g.wait_complete = false;
        }
        let h = g.pend;
        let Some(b) = g.bufs.iter_mut().find(|b| b.h == h) else {
            g.pend = 0;
            return;
        };
        b.busy = true;
        let pixmap = b.pixmap;
        let rects = if g.full { Vec::new() } else { core::mem::take(&mut g.damage) };
        g.damage.clear();
        g.full = false;
        g.pend = 0;
        g.newest = h;
        g.wait_complete = true;
        g.t0 = now;
        let mut update = 0;
        if self.g.xfixes_op != 0 && !rects.is_empty() {
            update = self.gpu_region(i, &rects);
        }
        self.g.serial = self.g.serial.wrapping_add(1);
        let serial = self.g.serial;
        self.xp[i].gpu.serial = serial;
        let xid = self.xw[i].win;
        let (o, _) = self.req(self.g.present_op | PRESENT_PIXMAP << 8, 72);
        let b = &mut self.out;
        wr32(b, o, xid);
        wr32(b, o + 4, pixmap);
        wr32(b, o + 8, serial);
        wr32(b, o + 12, 0); // valid: all of the pixmap
        wr32(b, o + 16, update);
        // x, y 0; crtc, fences none; options none (the server's next chance, no tearing);
        // target msc, divisor, remainder 0: the next vertical blank
        self.flush();
    }

    /// the window's region set to these rectangles (x, y, w, h)
    fn gpu_region(&mut self, i: usize, rects: &[[i32; 4]]) -> u32 {
        let mut region = self.xp[i].gpu.region;
        let minor = if region == 0 { XFIXES_CREATE_REGION } else { XFIXES_SET_REGION };
        if region == 0 {
            region = self.new_id();
            self.xp[i].gpu.region = region;
        }
        let (o, _) = self.req(self.g.xfixes_op | minor << 8, 8 + rects.len() * 8);
        let b = &mut self.out;
        wr32(b, o, region);
        for (k, r) in rects.iter().enumerate() {
            let p = o + 4 + k * 8;
            wr16(b, p, r[0] as u32);
            wr16(b, p + 2, r[1] as u32);
            wr16(b, p + 4, r[2] as u32);
            wr16(b, p + 6, r[3] as u32);
        }
        region
    }

    /// (present_service) a window on buffers -> its next deadline
    pub(super) fn gpu_service(&mut self, i: usize, now: u64) -> u64 {
        if self.xp[i].gpu.pend == 0 {
            return u64::MAX;
        }
        self.gpu_try(i, now);
        let g = &self.xp[i].gpu;
        if g.pend != 0 && g.wait_complete { g.t0 + SAFETY } else { u64::MAX }
    }

    pub(super) fn gpu_next_look(&mut self, w: i32) -> u64 {
        let now = crate::time_us();
        let Some(i) = self.rec(w) else { return now + IDLE_LOOK };
        let g = &self.xp[i].gpu;
        if g.wait_complete { (g.t0 + SAFETY).min(now + IDLE_LOOK * 10) } else { now + IDLE_LOOK }
    }

    /// the socket, whatever the event queue holds
    pub(super) fn gpu_pump(&mut self, ms: i32) {
        self.dispatch();
        self.present_service(crate::time_us());
        self.flush();
        if self.poll(POLLIN, ms) & POLLIN != 0 {
            self.recv();
        }
        self.dispatch();
        self.present_service(crate::time_us());
        self.flush();
    }

    /// (Expose) window i shows through buffers: that area again from the newest -> done
    pub(super) fn gpu_expose(&mut self, i: usize, x: i32, y: i32, w: i32, h: i32) -> bool {
        let g = &self.xp[i].gpu;
        if !g.active {
            return false;
        }
        let Some(b) = g.bufs.iter().find(|b| b.h == g.newest) else { return true };
        let (pixmap, xid, gc) = (b.pixmap, self.xw[i].win, self.gc);
        let (o, _) = self.req(XR_COPY_AREA, 28);
        let b = &mut self.out;
        wr32(b, o, pixmap);
        wr32(b, o + 4, xid);
        wr32(b, o + 8, gc);
        for (k, v) in [x, y, x, y, w, h].into_iter().enumerate() {
            wr16(b, o + 12 + k * 2, v as u32);
        }
        true
    }

    /// a Present event (GenericEvent of its major opcode)
    pub(super) fn gpu_event(&mut self, b: &[u8]) {
        if b.len() < 32 {
            return;
        }
        let i = self.find(rd32(b, 16));
        if i < 0 {
            return;
        }
        let g = &mut self.xp[i as usize].gpu;
        match rd16(b, 8) {
            PRESENT_EV_COMPLETE => {
                if b.len() >= 40 && rd32(b, 20) == g.serial {
                    g.wait_complete = false;
                }
            }
            PRESENT_EV_IDLE => {
                let pixmap = rd32(b, 24);
                if let Some(x) = g.bufs.iter_mut().find(|x| x.pixmap == pixmap) {
                    x.busy = false;
                }
            }
            _ => return,
        }
        if core::mem::take(&mut g.want) {
            self.g.wake = true;
        }
    }

    /// (tests) what the server holds for the window
    pub(super) fn gpu_test_image(&mut self, w: i32) -> Option<(i32, i32, Vec<u8>)> {
        let i = self.rec(w)?;
        let (ww, wh) = (self.xw[i].w, self.xw[i].h);
        let s = self.image_of(w, [0, 0, ww, wh]).ok()?;
        Some((s.w, s.h, s.px))
    }
}
