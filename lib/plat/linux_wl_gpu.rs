// linux_wl_gpu -- GPU buffers on a wl_surface: zwp_linux_dmabuf_v1 (programs against glibc;
// linux_gpuout.rs says what the calls mean and why no wrong frame can show)
//
//   gpu_caps()            what the compositor takes: the default feedback (version 4 on), its
//                         format table, the tranches of its main device; below that the modifier
//                         events of version 3.  Below version 3: nothing, the CPU path.  Asked at the
//                         first call, not at the start of the program
//   gpu_buf_new           zwp_linux_buffer_params_v1.create, not create_immed: a buffer the
//                         compositor cannot import gets `failed` and the CPU path, where create_immed
//                         may end the whole connection
//   gpu_show, gpu_try     attach + damage + frame callback + commit through the code of the shared
//                         memory path (the configure's acknowledgement, window geometry, buffer scale,
//                         viewport: linux_wl_present.rs surface_state, commit_buffer).  Shared memory
//                         and dmabufs take turns on one wl_surface, no child surface is needed: a
//                         wl_buffer is a wl_buffer (KWin, checked by `gpu expect` and gputest)
//   gpu_service           present_service for a window on buffers: a commit that waited, a window
//                         mapped again gets its newest buffer
//
// Pacing as on the shared memory path: one commit per frame callback (1 s at most).  gpu_can_show
// says so to the frame that wants to start, the frame stays in the presenter's texture and its newest
// state is drawn when the callback arrives: nothing queues up.  The event loop gets back from wait()
// when the callback or a buffer's release arrives for a frame that waits (Dm::wake).

use alloc::vec::Vec;

use rt::os::{syscall6, SYS_MMAP, SYS_MUNMAP};

use super::present::FRAME_SAFETY;
use super::proto::*;
use super::wire::{sys_close, Args, A};
use super::Wl;
use crate::{GpuBufDesc, GpuCaps, GPU_BUF_BUSY, GPU_BUF_FAILED, GPU_BUF_FREE, GPU_BUF_PENDING, PW_MAX};

/// an answer that never comes: the frame looks again after this long (us)
const RELEASE_LOOK: u64 = 100_000;
/// the rectangles a commit names, then only their bounding box
const DAMAGE_MAX: usize = 16;

const DM_CREATE_PARAMS: Req = Req { op: 1, sig: "create_params:n", destroy: false };
const DM_GET_DEFAULT_FEEDBACK: Req = Req { op: 2, sig: "get_default_feedback:n", destroy: false };
const FB_DESTROY: Req = Req { op: 0, sig: "destroy:", destroy: true };
const PARAMS_DESTROY: Req = Req { op: 0, sig: "destroy:", destroy: true };
const PARAMS_ADD: Req = Req { op: 1, sig: "add:huuuuu", destroy: false };
const PARAMS_CREATE: Req = Req { op: 2, sig: "create:iiuu", destroy: false };

pub(crate) static IFACE_DMABUF: Iface = Iface {
    name: "zwp_linux_dmabuf_v1",
    evs: &[Ev { sig: "format:u", h: None }, Ev { sig: "modifier:uuu", h: Some(wlh_dm_modifier) }],
};

static IFACE_FEEDBACK: Iface = Iface {
    name: "zwp_linux_dmabuf_feedback_v1",
    evs: &[
        Ev { sig: "done:", h: Some(wlh_fb_done) },
        Ev { sig: "format_table:hu", h: Some(wlh_fb_table) },
        Ev { sig: "main_device:a", h: Some(wlh_fb_main) },
        Ev { sig: "tranche_done:", h: Some(wlh_fb_tranche_done) },
        Ev { sig: "tranche_target_device:a", h: Some(wlh_fb_target) },
        Ev { sig: "tranche_formats:a", h: Some(wlh_fb_formats) },
        Ev { sig: "tranche_flags:u", h: None },
    ],
};

static IFACE_PARAMS: Iface = Iface {
    name: "zwp_linux_buffer_params_v1",
    evs: &[Ev { sig: "created:n", h: Some(wlh_params_created) }, Ev { sig: "failed:", h: Some(wlh_params_failed) }],
};

const PRESENTATION_FEEDBACK: Req = Req { op: 1, sig: "feedback:on", destroy: false };

static IFACE_PRESENTATION: Iface = Iface { name: "wp_presentation", evs: &[Ev { sig: "clock_id:u", h: None }] };

static IFACE_PRESENTATION_FB: Iface = Iface {
    name: "wp_presentation_feedback",
    evs: &[
        Ev { sig: "sync_output:o", h: None },
        Ev { sig: "presented:uuuuuuu", h: Some(wlh_presented) },
        Ev { sig: "discarded:", h: Some(wlh_discarded) },
    ],
};

static IFACE_GPU_BUFFER: Iface = Iface { name: "wl_buffer", evs: &[Ev { sig: "release:", h: Some(wlh_gpu_release) }] };

/// the compositor's side, one per connection
#[derive(Default)]
pub(crate) struct Dm {
    pub mgr: u32,
    pub ver: u32,
    fb: u32,
    asked: bool,
    done: bool,
    table: Vec<(u32, u64)>,
    main_dev: Option<u64>,
    tr_dev: Option<u64>,
    tr_fmt: Vec<u16>,
    formats: Vec<(u32, u64)>,
    next: u32,
    /// the loop in wait() goes back to its caller
    pub wake: bool,
    /// (bench) wp_presentation, what it said
    pres: u32,
    presented: Vec<(u64, u64)>,
}

struct Buf {
    h: u32,
    id: u32, // wl_buffer (0: its answer has not come)
    params: u32,
    failed: bool,
    busy: bool,
    w: i32,
    ph: i32,
}

/// a window's buffers (in its presentation record)
#[derive(Default)]
pub(crate) struct WpGpu {
    pub active: bool,
    bufs: Vec<Buf>,
    newest: u32,
    pend: u32,
    full: bool,
    damage: Vec<[i32; 4]>,
    want: bool,
}

impl WpGpu {
    /// the CPU path takes over: the frame on its way is dropped
    pub(crate) fn leave(&mut self) {
        self.active = false;
        self.pend = 0;
        self.newest = 0;
        self.damage.clear();
        self.full = false;
    }

    /// the surface was destroyed: nothing is attached any more
    pub(crate) fn unmapped(&mut self) {
        for b in self.bufs.iter_mut() {
            b.busy = false;
        }
    }

    fn add_damage(&mut self, rects: Option<&[[i32; 4]]>) {
        match rects {
            Some(r) if !self.full => {
                self.damage.extend_from_slice(r);
                if self.damage.len() > DAMAGE_MAX {
                    self.full = true;
                }
            }
            _ => self.full = true,
        }
    }
}

impl Wl {
    /// what the compositor takes  [no assembly: G4b]
    pub(crate) fn gpu_caps(&mut self) -> Option<GpuCaps> {
        if self.dm.mgr == 0 {
            return None;
        }
        if !self.dm.asked {
            self.dm.asked = true;
            if self.dm.ver >= 4 {
                self.dm.fb = self.w.send(self.dm.mgr, &DM_GET_DEFAULT_FEEDBACK, &[A::N(&IFACE_FEEDBACK)]);
            }
            if self.roundtrip() != 0 {
                return None;
            }
        }
        Some(GpuCaps { device: self.dm.main_dev, formats: self.dm.formats.clone() })
    }

    pub(crate) fn gpu_buf_new(&mut self, w: i32, d: &GpuBufDesc) -> u32 {
        let Some(i) = self.rec(w).filter(|_| self.dm.mgr != 0 && !d.planes.is_empty()) else {
            crate::linux::gpuout::close_planes(d);
            return 0;
        };
        self.dm.next = self.dm.next.wrapping_add(1).max(1);
        let h = self.dm.next;
        let params = self.w.send(self.dm.mgr, &DM_CREATE_PARAMS, &[A::N(&IFACE_PARAMS)]);
        self.w.set_udata(params, (i as u64) << 32 | h as u64);
        for (k, p) in d.planes.iter().enumerate() {
            self.w.send(params, &PARAMS_ADD, &[A::H(p.fd), A::U(k as u32), A::U(p.offset), A::U(p.stride),
                                               A::U((d.modifier >> 32) as u32), A::U(d.modifier as u32)]);
        }
        self.w.send(params, &PARAMS_CREATE, &[A::I(d.w), A::I(d.h), A::U(d.fourcc), A::U(0)]);
        self.w.flush();
        self.wp[i].gpu.bufs.push(Buf { h, id: 0, params, failed: false, busy: false, w: d.w, ph: d.h });
        h
    }

    pub(crate) fn gpu_buf_state(&mut self, w: i32, h: u32) -> u32 {
        let Some(i) = self.rec(w) else { return GPU_BUF_FAILED };
        let g = &mut self.wp[i].gpu;
        let (newest, pend) = (g.newest, g.pend);
        let Some(b) = g.bufs.iter().find(|b| b.h == h) else { return GPU_BUF_FAILED };
        if b.failed {
            return GPU_BUF_FAILED;
        }
        if b.id == 0 {
            return GPU_BUF_PENDING;
        }
        if b.busy || h == newest || h == pend {
            g.want = true;
            return GPU_BUF_BUSY;
        }
        GPU_BUF_FREE
    }

    pub(crate) fn gpu_buf_free(&mut self, w: i32, h: u32) {
        let Some(i) = self.rec(w) else { return };
        let g = &mut self.wp[i].gpu;
        let Some(k) = g.bufs.iter().position(|b| b.h == h) else { return };
        let b = g.bufs.swap_remove(k);
        if g.newest == h {
            g.newest = 0;
        }
        if g.pend == h {
            g.pend = 0;
        }
        if b.id != 0 {
            self.w.send(b.id, &BUFFER_DESTROY, &[]);
            self.w.flush();
        }
        // (no answer yet: wlh_params_created destroys it, its record is gone)
    }

    /// the window is destroyed: its buffers go
    pub(crate) fn gpu_free_all(&mut self, i: usize) {
        for b in core::mem::take(&mut self.wp[i].gpu.bufs) {
            if b.id != 0 {
                self.w.send(b.id, &BUFFER_DESTROY, &[]);
            }
        }
    }

    pub(crate) fn gpu_can_show(&mut self, w: i32) -> bool {
        let Some(i) = self.rec(w) else { return false };
        let r = &self.win[i];
        if !r.shown || !r.configured {
            return false;
        }
        let now = crate::time_us();
        let wp = &mut self.wp[i];
        if wp.gpu.pend != 0 || (wp.frame_wait && now.wrapping_sub(wp.frame_t0) < FRAME_SAFETY) {
            wp.gpu.want = true;
            return false;
        }
        true
    }

    pub(crate) fn gpu_show(&mut self, w: i32, h: u32, rects: Option<&[[i32; 4]]>) {
        let Some(i) = self.rec(w) else { return };
        if !self.wp[i].gpu.bufs.iter().any(|b| b.h == h && b.id != 0) {
            return;
        }
        if !self.wp[i].gpu.active {
            // the shared memory path's waiting frame is over, buffers show from now on
            self.present_gpu_begin(i);
            self.wp[i].gpu.active = true;
            self.wp[i].gpu.full = true;
            self.wp[i].gpu.damage.clear();
        }
        self.wp[i].gpu.pend = h;
        self.wp[i].gpu.add_damage(rects);
        self.gpu_try(i, crate::time_us());
    }

    /// the waiting buffer goes on the window when the window can take it
    fn gpu_try(&mut self, i: usize, now: u64) {
        let r = self.win[i];
        if self.wp[i].gpu.pend == 0 || !r.shown || !r.configured {
            return;
        }
        let wp = &mut self.wp[i];
        if wp.frame_wait {
            if now.wrapping_sub(wp.frame_t0) < FRAME_SAFETY {
                return;
            }
            wp.frame_wait = false;
        }
        let h = wp.gpu.pend;
        let Some(b) = wp.gpu.bufs.iter().find(|b| b.h == h) else {
            wp.gpu.pend = 0;
            return;
        };
        let (id, bw, bh) = (b.id, b.w, b.ph);
        if !self.surface_state(i, bw, bh) {
            return; // integer scale: the size must be a multiple; the frame of the new size follows
        }
        let g = &mut self.wp[i].gpu;
        let boxes: Vec<[i32; 4]> = if g.full {
            alloc::vec![[0, 0, bw, bh]]
        } else {
            g.damage.iter().map(|r| [r[0], r[1], r[0] + r[2], r[1] + r[3]]).collect()
        };
        g.damage.clear();
        g.full = false;
        g.pend = 0;
        g.newest = h;
        if let Some(b) = g.bufs.iter_mut().find(|b| b.h == h) {
            b.busy = true;
        }
        self.commit_buffer(i, id, &boxes, now);
    }

    /// (present_service) a window on buffers -> its next deadline
    pub(crate) fn gpu_service(&mut self, i: usize, now: u64) -> u64 {
        // mapped again: the whole last frame
        if !self.win[i].mapped && self.wp[i].gpu.newest != 0 && self.wp[i].gpu.pend == 0 {
            let g = &mut self.wp[i].gpu;
            g.pend = g.newest;
            g.full = true;
        }
        if self.wp[i].gpu.pend == 0 {
            return u64::MAX;
        }
        self.gpu_try(i, now);
        let wp = &self.wp[i];
        if wp.gpu.pend != 0 && wp.frame_wait { wp.frame_t0 + FRAME_SAFETY } else { u64::MAX }
    }

    pub(crate) fn gpu_next_look(&mut self, w: i32) -> u64 {
        let now = crate::time_us();
        let Some(i) = self.rec(w) else { return now + RELEASE_LOOK };
        let wp = &self.wp[i];
        if wp.frame_wait { (wp.frame_t0 + FRAME_SAFETY).min(now + RELEASE_LOOK * 10) } else { now + RELEASE_LOOK }
    }

    /// (test aid) as if the compositor preferred the scale of this dpi
    pub(crate) fn gpu_test_dpi(&mut self, w: i32, dpi: i32) -> i64 {
        let Some(i) = self.rec(w) else { return -1 };
        if self.win[i].vp == 0 {
            return -1;
        }
        self.win[i].frac_pref = (dpi as u32 * 120).div_ceil(96);
        self.update_scale(w);
        1
    }

    /// (test aid) what the compositor's next activation does: the state goes
    pub(crate) fn gpu_test_unminimize(&mut self, w: i32) {
        if let Some(i) = self.rec(w) {
            self.win[i].state &= !crate::WST_MINIMIZED;
        }
    }

    /// (test aid) popup slot `slot` has a surface: 1, else -1
    pub(crate) fn gpu_test_popup(&self, slot: i32) -> i64 {
        if self.popup_shown(slot) { 1 } else { -1 }
    }

    /// (bench) wp_presentation: bound when the variable says so
    pub(crate) fn gpu_feedback_setup(&mut self, name: u32, ver: u32) {
        if self.dm.pres == 0 && crate::env("jfgui_WL_FEEDBACK").is_some() {
            self.dm.pres = self.w.send(self.g.reg, &REGISTRY_BIND, &[A::U(name), A::B(&IFACE_PRESENTATION, ver.min(1))]);
        }
    }

    /// (bench) a commit of window i follows: when does it reach the screen
    pub(crate) fn gpu_feedback(&mut self, surf: u32, now: u64) {
        if self.dm.pres != 0 {
            let fb = self.w.send(self.dm.pres, &PRESENTATION_FEEDBACK, &[A::O(surf), A::N(&IFACE_PRESENTATION_FB)]);
            self.w.set_udata(fb, now);
        }
    }

    pub(crate) fn gpu_test_presented(&mut self) -> Vec<(u64, u64)> {
        core::mem::take(&mut self.dm.presented)
    }

    /// the socket, whatever the event queue holds
    pub(crate) fn gpu_pump(&mut self, ms: i32) {
        self.dispatch();
        self.present_service(crate::time_us());
        self.w.flush();
        if self.w.wait_readable(ms) {
            self.w.recv();
        }
        self.dispatch();
        self.present_service(crate::time_us());
        self.w.flush();
    }
}

fn wlh_dm_modifier(wl: &mut Wl, _id: u32, a: &Args, _u: u64) {
    if wl.dm.ver < 4 {
        let e = (a.u(0), (a.u(1) as u64) << 32 | a.u(2) as u64);
        if !wl.dm.formats.contains(&e) {
            wl.dm.formats.push(e);
        }
    }
}

fn dev_t(a: &[u8]) -> Option<u64> {
    a.get(..8).and_then(|b| b.try_into().ok()).map(u64::from_le_bytes)
}

fn wlh_fb_main(wl: &mut Wl, _id: u32, a: &Args, _u: u64) {
    wl.dm.main_dev = dev_t(a.arr(0));
}

fn wlh_fb_target(wl: &mut Wl, _id: u32, a: &Args, _u: u64) {
    wl.dm.tr_dev = dev_t(a.arr(0));
}

/// the table: 16 bytes per entry (format, padding, modifier), mapped read-only
fn wlh_fb_table(wl: &mut Wl, _id: u32, a: &Args, _u: u64) {
    let (fd, size) = (a.v[0] as u32 as i32, a.u(1) as usize);
    // SAFETY: a private read-only mapping of the file the compositor sent, size bytes.
    let map = unsafe { syscall6(SYS_MMAP, 0, size, 1, 2, fd as usize, 0) };
    if (-4095..0).contains(&map) {
        sys_close(fd);
        return;
    }
    // SAFETY: the mapping made above.
    let b = unsafe { core::slice::from_raw_parts(map as *const u8, size) };
    wl.dm.table = b.chunks_exact(16).map(|e| {
        (u32::from_le_bytes([e[0], e[1], e[2], e[3]]), u64::from_le_bytes([e[8], e[9], e[10], e[11], e[12], e[13], e[14], e[15]]))
    }).collect();
    // SAFETY: the mapping made above, not used any more.
    unsafe { syscall6(SYS_MUNMAP, map as usize, size, 0, 0, 0, 0) };
    sys_close(fd);
}

fn wlh_fb_formats(wl: &mut Wl, _id: u32, a: &Args, _u: u64) {
    wl.dm.tr_fmt.extend(a.arr(0).chunks_exact(2).map(|b| u16::from_le_bytes([b[0], b[1]])));
}

/// a tranche for the compositor's main device: what it can read with the GPU we want
fn wlh_fb_tranche_done(wl: &mut Wl, _id: u32, _a: &Args, _u: u64) {
    let d = &mut wl.dm;
    if d.tr_dev == d.main_dev || d.tr_dev.is_none() {
        for &k in &d.tr_fmt {
            if let Some(&e) = d.table.get(k as usize) {
                if !d.formats.contains(&e) {
                    d.formats.push(e);
                }
            }
        }
    }
    d.tr_fmt.clear();
    d.tr_dev = None;
}

fn wlh_fb_done(wl: &mut Wl, id: u32, _a: &Args, _u: u64) {
    wl.dm.done = true;
    wl.dm.table = Vec::new();
    // (what comes later, another GPU for instance, is not followed: a new swapchain asks nobody)
    wl.w.send(id, &FB_DESTROY, &[]);
    wl.dm.fb = 0;
}

fn wlh_params_created(wl: &mut Wl, id: u32, a: &Args, u: u64) {
    let (i, h) = ((u >> 32) as usize & (PW_MAX - 1), u as u32);
    let buf = a.u(0);
    wl.w.srv_add(buf, &IFACE_GPU_BUFFER, i as u64);
    wl.w.send(id, &PARAMS_DESTROY, &[]);
    match wl.wp[i].gpu.bufs.iter_mut().find(|b| b.h == h && b.params == id) {
        Some(b) => {
            b.id = buf;
            b.params = 0;
            wl.dm.wake = true;
        }
        None => {
            wl.w.send(buf, &BUFFER_DESTROY, &[]); // it went meanwhile
        }
    }
}

fn wlh_params_failed(wl: &mut Wl, id: u32, _a: &Args, u: u64) {
    let (i, h) = ((u >> 32) as usize & (PW_MAX - 1), u as u32);
    wl.w.send(id, &PARAMS_DESTROY, &[]);
    if let Some(b) = wl.wp[i].gpu.bufs.iter_mut().find(|b| b.h == h && b.params == id) {
        b.failed = true;
        b.params = 0;
        wl.dm.wake = true;
    }
}

fn wlh_gpu_release(wl: &mut Wl, id: u32, _a: &Args, u: u64) {
    let g = &mut wl.wp[u as usize & (PW_MAX - 1)].gpu;
    if let Some(b) = g.bufs.iter_mut().find(|b| b.id == id) {
        b.busy = false;
    }
    if core::mem::take(&mut g.want) {
        wl.dm.wake = true;
    }
}

/// (wlh_frame_done) a frame that waits for the callback may start now
pub(crate) fn frame_done(wl: &mut Wl, i: usize) {
    if core::mem::take(&mut wl.wp[i].gpu.want) {
        wl.dm.wake = true;
    }
}

/// (bench) wp_presentation_feedback: on the screen (the clock is CLOCK_MONOTONIC, as time_us)
fn wlh_presented(wl: &mut Wl, _id: u32, a: &Args, u: u64) {
    let sec = (a.u(0) as u64) << 32 | a.u(1) as u64;
    wl.dm.presented.push((u, sec * 1_000_000 + a.u(2) as u64 / 1000));
}

fn wlh_discarded(wl: &mut Wl, _id: u32, _a: &Args, u: u64) {
    wl.dm.presented.push((u, 0));
}
