// uigputest -- the UI's output through the GPU with real windows (lib/ui/gpu_out.rs, docs/GPU.md
// "Ausgabe der UI")
//
//   uigputest --test NAME     one test: a real window, the real window loop (ui::run_window),
//                             shown without activation, no real input; a line per check, exit
//                             code 1 when one fails.  Without a usable GPU: SKIP, passed.  A check of
//                             the screen where another window lies on top of ours is a note, no
//                             result.
//     switch    the first frames on the CPU path, then the swapchain; the screen never shows
//               anything else than the picture meanwhile; a small change uploads little; every
//               presented image equals the framebuffer
//     resize    SetWindowPos and the modal size loop: CPU frames (the screen follows), the swapchain
//               again when the size has settled
//     minimize  minimised: nothing uploaded or presented; restored: what changed meanwhile shows
//     dpi       a DPI change: the bigger frame, the swapchain again
//     windows   a second window on the same device; a popup outside the window stays on the CPU
//               path; the second window closes and comes back
//     off       jfgui_GPU=0: the CPU path, the window's DC shows the frame
//     noloader  jfgui_GPU_DLL names a missing file: the CPU path, no retry, no message
//     lose      Device::test_lose: the frame back through the CPU path and on the screen (GDI
//               shows again: the swapchain lives on a child), a new device after a while
//     idle      no frame and hardly any CPU time while nothing changes; a caret blink is one small
//               upload
//     csd       a window with the library's frame (AF_CUSTOM_TITLEBAR): the resize band answers
//               through the child, a new size and back on the swapchain (maximise and full screen
//               take the activation, so no test does them; by hand they go the same way)
//     monitors  (with two monitors, else SKIP) moved to the other monitor: the frames go on
//     shaders   wgsl::compile of lib/ui/gpu_out.wgsl = the embedded SPIR-V, word for word
//     adapter   the ranking of adapters for the UI; on this machine the adapter of the display
//     hwnd      (information, no failure) a swapchain directly on a window: GDI before it, GDI
//               after it was destroyed: why lib/ui's swapchains live on a child window
//   uigputest --gen-shaders FILE   lib/ui/gpu_out_spv.rs from lib/ui/gpu_out.wgsl
//   uigputest --bench latency [--scene small|full] [--n N] [--burst]   event -> present returned and
//                             event -> the screen changed (a thread posts key presses and reads the
//                             screen); jfgui_GPU=0|1, jfgui_GPU_PRESENT choose the path
//   uigputest --bench idle|caret      a window with nothing to do / with a blinking caret, until
//                             --auto-close: frames and uploads on stdout (the process's counters
//                             are read from outside)
//
// Every test runs under VK_LAYER_KHRONOS_validation when it is installed (a message fails the
// test) and with jfgui_GPU_VERIFY=1, except where a test says otherwise.  --plain: neither.
#![no_std]
#![no_main]
#![deny(unsafe_code)]

extern crate alloc;

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

use ui::*;

const DESC: AppDesc = AppDesc { title: "uigputest", flags: AF_RESIZABLE, w: 640, h: 480, min_w: 200, min_h: 150, ..AppDesc::NONE };

const BANDS: [u32; 6] = [0xFF20_3040, 0xFF40_6080, 0xFF80_4020, 0xFF10_7050, 0xFF70_1060, 0xFF50_5050];
const WM_KEYDOWN: u32 = 0x0100;
const WM_KEYUP: u32 = 0x0101;
const WM_ENTERSIZEMOVE: u32 = 0x0231;
const WM_EXITSIZEMOVE: u32 = 0x0232;
const VK_F9: u32 = 0x78;
const SW_SHOWMINNOACTIVE: i32 = 7;
const SW_SHOWNOACTIVATE: i32 = 4;

fn out(s: &str) {
    plat::write_stdout(s.as_bytes());
    plat::write_stdout(b"\n");
}

fn screen_px(x: i32, y: i32) -> u32 {
    plat::win_test_screen(x, y, 1, 1).map_or(0xDEAD_0000, |v| v[0] as u32 | (v[1] as u32) << 8 | (v[2] as u32) << 16)
}

/// a thread watches points of the screen: everything but the expected colours counts
struct Watch {
    stop: AtomicBool,
    samples: AtomicU32,
    bad: AtomicU32,
    covered: AtomicU32,
    last_bad: AtomicU32,
}

struct Test {
    name: String,
    fails: u32,
    pc: u32,
    since: u64,
    patch: u32,
    full: u32,
    done: bool,
    t_first: bool,
    started: bool,
    passes: u64,
    since_pass: u64,
    watch: Option<Arc<Watch>>,
    mark: [u64; 4],
    second: Shared<i32>,
    text: String,
    caret: bool,
    mode: u32,
    bench: Option<Arc<Bench>>,
}

fn band_of(x: i32, w: i32) -> u32 {
    BANDS[(x * 6 / w.max(1)).clamp(0, 5) as usize]
}

fn bgr(c: u32) -> u32 {
    c & 0xFF_FFFF
}

impl Test {
    fn ok(&mut self, cond: bool, what: &str) -> bool {
        if cond {
            out(&format!("  ok   {what}"));
        } else {
            self.fails += 1;
            out(&format!("FAIL {what}"));
        }
        cond
    }

    /// the main window, not its child, is found at a client point (a window of another program on
    /// top of ours is a note, no result)
    fn finds_window(&mut self, x: i32, y: i32, what: &str) {
        match plat::win_test(plat::WT_FROM_POINT, 0, [x, y, 0, 0]) {
            0 => out(&format!("note: {what}: another window lies on top of ours (no result)")),
            at => {
                self.ok(at == 1, what);
            }
        }
    }

    fn next(&mut self) {
        self.pc += 1;
        self.since = plat::time_us();
        self.since_pass = self.passes;
    }

    /// the step has waited ms, and what it changed has been drawn (a pass and its frame followed,
    /// also when this process got no time for a while on a busy machine)
    fn after(&self, ms: u64) -> bool {
        plat::time_us() >= self.since + ms * 1000 && self.passes >= self.since_pass + 2
    }

    fn finish(&mut self, ui: &mut Ui) {
        if let Some(w) = self.watch.take() {
            w.stop.store(true, Ordering::Relaxed);
        }
        let (e, w) = gpu::validation_messages();
        if plat::env("jfgui_GPU_VALIDATE").is_some() {
            self.ok(e == 0, &format!("validation layer: {e} errors, {w} warnings"));
        }
        let o = ui.gpu_output(0);
        if o.verified != 0 {
            self.ok(o.verify_bad == 0, &format!("{} presented images read back, {} differ", o.verified, o.verify_bad));
        }
        self.done = true;
        ui.quit();
    }

    /// waits for the swapchain -> Some(true) when on; a failure ends the test
    fn wait_on(&mut self, ui: &mut Ui, ms: u64) -> Option<bool> {
        let o = ui.gpu_output(0);
        if o.never {
            out(&format!("SKIP: no GPU output: {}", o.error.as_deref().unwrap_or("")));
            self.done = true;
            ui.quit();
            return None;
        }
        if o.on {
            return Some(true);
        }
        if self.after(ms) {
            self.ok(false, &format!("the swapchain did not come: {:?}", o.error));
            self.finish(ui);
            return None;
        }
        Some(false)
    }

    /// the picture's colour at client pixel x, y
    fn expected(&self, ui: &Ui, x: i32, y: i32) -> u32 {
        let fb = ui.fb();
        if self.full != 0 {
            return bgr(self.full);
        }
        if (100..300).contains(&x) && (100..140).contains(&y) {
            return bgr(self.patch);
        }
        bgr(band_of(x, fb.w))
    }

    /// a strip of the window on the screen, below the patch: it shows the bands, or else
    fn watch_start(&mut self, ui: &Ui) {
        let Some(r) = plat::win_get_rect(0) else { return };
        let fb = ui.fb();
        let (x0, y0, w) = (r[0], r[1] + fb.h - 8, fb.w);
        let want: Vec<u32> = (0..w).map(|x| bgr(band_of(x, w))).collect();
        let watch = Arc::new(Watch { stop: AtomicBool::new(false), samples: AtomicU32::new(0), bad: AtomicU32::new(0), covered: AtomicU32::new(0), last_bad: AtomicU32::new(0) });
        let w2 = watch.clone();
        let ours = plat::win_native(0).map_or(0, |n| n.hwnd);
        let th = plat::thread_spawn(0, Box::new(move || {
            while !w2.stop.load(Ordering::Relaxed) {
                let Some(px) = plat::win_test_screen(x0, y0, w, 1) else { continue };
                w2.samples.fetch_add(1, Ordering::Relaxed);
                let wrong = px.chunks_exact(4).zip(want.iter()).find(|(p, c)| (p[0] as u32 | (p[1] as u32) << 8 | (p[2] as u32) << 16) != **c);
                if let Some((p, c)) = wrong {
                    let i = (p.as_ptr() as usize - px.as_ptr() as usize) / 4;
                    let v = p[0] as u32 | (p[1] as u32) << 8 | (p[2] as u32) << 16;
                    if plat::win_test_window_at(x0 + i as i32, y0) != ours {
                        w2.covered.fetch_add(1, Ordering::Relaxed); // another window on top of ours
                        continue;
                    }
                    if w2.bad.fetch_add(1, Ordering::Relaxed) < 6 {
                        out(&format!("  screen: t={} x={i} {v:06x}, not {c:06x}", plat::time_us() / 1000 % 100000));
                    }
                    w2.last_bad.store(v, Ordering::Relaxed);
                }
            }
        }));
        drop(th);
        self.watch = Some(watch);
    }

    /// what the watcher saw (samples where another window lay on top of ours do not count)
    fn watch_result(&mut self, what: &str) {
        let Some(w) = self.watch.take() else { return };
        w.stop.store(true, Ordering::Relaxed);
        let (n, b, c) = (w.samples.load(Ordering::Relaxed), w.bad.load(Ordering::Relaxed), w.covered.load(Ordering::Relaxed));
        if c != 0 {
            out(&format!("note: {what}: another window lay on top of ours in {c} of {n} samples"));
        }
        self.ok(b == 0, &format!("{what}: {n} screen samples, {b} wrong (last {:06x})", w.last_bad.load(Ordering::Relaxed)));
    }

    /// the screen at the patch shows c by now (a window covering ours: a note)
    fn screen_check(&mut self, ui: &Ui, what: &str) {
        let Some(r) = plat::win_get_rect(0) else { return };
        let want = self.expected(ui, 150, 120);
        // what was presented reaches the screen with the next compositions; a busy machine takes
        // longer
        let t0 = plat::time_us();
        let mut got = screen_px(r[0] + 150, r[1] + 120);
        while got != want && plat::time_us() < t0 + 1_500_000 {
            plat::sleep_ms(4);
            got = screen_px(r[0] + 150, r[1] + 120);
        }
        if got != want && plat::win_test(plat::WT_FROM_POINT, 0, [150, 120, 0, 0]) != 1 {
            out(&format!("note: {what}: another window lies on top of ours (no result)"));
            return;
        }
        self.ok(got == want, &format!("{what}: the screen shows {got:06x}, expected {want:06x}"));
    }

    // ---------------------------------------------------------------- the tests

    fn t_switch(&mut self, ui: &mut Ui) {
        match self.pc {
            0 => {
                // the first frame is drawn right after this pass: nothing of the GPU waits for it
                let o = ui.gpu_output(0);
                self.ok(!o.on && !o.building, "first pass: no swapchain yet, nothing started");
                self.next();
            }
            1 => {
                if self.after(40) {
                    self.watch_start(ui);
                    self.next();
                }
            }
            2 => {
                let t = plat::time_us() - self.since;
                match self.wait_on(ui, 15_000) {
                    Some(true) => {
                        out(&format!("  swapchain after {:.1} ms at t={}", t as f64 / 1000.0, plat::time_us() / 1000 % 100000));
                        let o = ui.gpu_output(0);
                        out(&format!("  {} ({}), {} MiB reserved", o.adapter, o.present_mode, o.reserved >> 20));
                        self.ok(o.switches == 1 && o.cpu_frames >= 1, &format!("{} CPU frame(s), then the switch", o.cpu_frames));
                        self.mark[0] = o.upload_bytes;
                        self.mark[1] = o.frames;
                        self.next();
                    }
                    _ => {}
                }
            }
            3 => {
                if self.after(200) {
                    self.watch_result("the screen during the switch");
                    self.finds_window(320, 240, "the pointer finds the window, not the child on it");
                    self.patch = 0xFF00_C0F0;
                    self.next();
                }
            }
            4 => {
                if self.after(100) {
                    let o = ui.gpu_output(0);
                    let up = o.upload_bytes - self.mark[0];
                    self.ok(o.frames == self.mark[1] + 1, &format!("a change: {} frame(s)", o.frames - self.mark[1]));
                    // the 32 px cells the 200 x 40 patch touches: at most 8 x 3 cells
                    self.ok(up > 0 && up <= (8 * 32 * 3 * 32 * 4) as u64, &format!("a change of 200 x 40 px uploaded {up} bytes"));
                    self.screen_check(ui, "the change");
                    self.patch = 0xFF00_40F0;
                    self.next();
                }
            }
            5 => {
                if self.after(100) {
                    self.screen_check(ui, "the next change");
                    // a burst: every pass a new colour, none waits for the display
                    self.mode = 1;
                    self.mark[0] = plat::time_us();
                    self.next();
                }
            }
            6 => {
                self.patch = self.patch.wrapping_add(0x10);
                ui.invalidate();
                if self.after(300) {
                    self.mode = 0;
                    let o = ui.gpu_output(0);
                    out(&format!("  burst: {} frames in 300 ms, frame pending: {}", o.frames - self.mark[1], o.pending));
                    self.next();
                }
            }
            7 => {
                if self.after(100) {
                    self.ok(!ui.gpu_output(0).pending, "after the burst no frame waits");
                    self.screen_check(ui, "after the burst");
                    self.finish(ui);
                }
            }
            _ => {}
        }
    }

    fn t_resize(&mut self, ui: &mut Ui) {
        let (x, y) = plat::win_get_rect(0).map_or((100, 100), |r| (r[0], r[1]));
        match self.pc {
            0 => {
                if self.wait_on(ui, 15_000) == Some(true) {
                    self.next();
                }
            }
            1 => {
                // like the user's snap: outside the modal loop
                plat::win_test(plat::WT_MOVE, 0, [x, y, 720, 500]);
                self.next();
            }
            2 => {
                let o = ui.gpu_output(0);
                if ui.fb().w == 720 {
                    if o.leaves == 1 {
                        self.ok(!o.on, "a new size: back on the CPU path");
                        self.next();
                    } else if self.after(1000) {
                        self.ok(false, "a new size: no CPU frame");
                        self.finish(ui);
                    }
                }
            }
            3 => {
                if self.wait_on(ui, 3000) == Some(true) {
                    self.ok(ui.gpu_output(0).switches == 2, "the swapchain again when the size has settled");
                    self.screen_check(ui, "after the resize");
                    self.next();
                }
            }
            4 => {
                // the modal size loop: every frame through the CPU path, the swapchain after it
                plat::win_test_send(0, WM_ENTERSIZEMOVE, 0, 0);
                self.mark[0] = 0;
                self.next();
            }
            5 | 6 | 7 | 8 => {
                let k = self.pc as i32 - 5;
                plat::win_test(plat::WT_MOVE, 0, [x, y, 600 + 37 * k, 420 + 23 * k]);
                self.next();
            }
            9 => {
                if self.after(300) {
                    let o = ui.gpu_output(0);
                    self.ok(!o.on && !o.building, "in the size loop: the CPU path, no swapchain being built");
                    self.ok(ui.fb().w == 600 + 37 * 3, "the frames follow the size");
                    self.screen_check(ui, "in the size loop");
                    plat::win_test_send(0, WM_EXITSIZEMOVE, 0, 0);
                    self.next();
                }
            }
            10 => {
                if self.wait_on(ui, 3000) == Some(true) {
                    self.ok(ui.gpu_output(0).switches == 3, "the swapchain again after the size loop");
                    self.patch = 0xFF30_D020;
                    self.next();
                }
            }
            11 => {
                if self.after(100) {
                    self.screen_check(ui, "a change after the size loop");
                    self.finish(ui);
                }
            }
            _ => {}
        }
    }

    fn t_minimize(&mut self, ui: &mut Ui) {
        match self.pc {
            0 => {
                if self.wait_on(ui, 15_000) == Some(true) {
                    plat::win_test(plat::WT_SHOW, 0, [SW_SHOWMINNOACTIVE, 0, 0, 0]);
                    self.next();
                }
            }
            1 => {
                if plat::win_state(0) & plat::WST_MINIMIZED != 0 {
                    let o = ui.gpu_output(0);
                    self.mark[0] = o.frames;
                    self.mark[1] = o.upload_bytes;
                    self.patch = 0xFFD0_A000;
                    self.next();
                } else if self.after(3000) {
                    self.ok(false, "the window did not minimise");
                    self.finish(ui);
                }
            }
            2 => {
                if self.after(150) {
                    let o = ui.gpu_output(0);
                    self.ok(o.frames == self.mark[0], "minimised: nothing presented");
                    self.ok(o.upload_bytes == self.mark[1], "minimised: nothing uploaded");
                    self.ok(o.pending, "minimised: a frame waits");
                    plat::win_test(plat::WT_SHOW, 0, [SW_SHOWNOACTIVATE, 0, 0, 0]);
                    self.next();
                }
            }
            3 => {
                if plat::win_state(0) & plat::WST_MINIMIZED == 0 && !ui.gpu_output(0).pending {
                    self.next();
                } else if self.after(3000) {
                    self.ok(false, "restored: the waiting frame was not shown");
                    self.finish(ui);
                }
            }
            4 => {
                if self.after(350) {
                    self.ok(ui.gpu_output(0).on, "restored: on the swapchain");
                    self.next();
                }
            }
            5 => {
                if self.after(100) {
                    self.screen_check(ui, "what changed while minimised");
                    self.finish(ui);
                }
            }
            _ => {}
        }
    }

    fn t_dpi(&mut self, ui: &mut Ui) {
        match self.pc {
            0 => {
                if self.wait_on(ui, 15_000) == Some(true) {
                    self.mark[0] = ui.fb().w as u64;
                    plat::win_test(plat::WT_DPI, 0, [144, 0, 0, 0]);
                    self.next();
                }
            }
            1 => {
                if ui.fb().w as u64 > self.mark[0] {
                    self.ok(true, "150 %: the bigger frame");
                    self.next();
                } else if self.after(2000) {
                    self.ok(false, &format!("150 %: the frame is {} wide", ui.fb().w));
                    self.finish(ui);
                }
            }
            2 => {
                if self.after(50) && self.wait_on(ui, 3000) == Some(true) {
                    self.ok(ui.gpu_output(0).switches == 2, "150 %: the swapchain again");
                    self.screen_check(ui, "150 %");
                    plat::win_test(plat::WT_DPI, 0, [96, 0, 0, 0]);
                    self.next();
                }
            }
            3 => {
                if self.after(50) && ui.fb().w as u64 == self.mark[0] && self.wait_on(ui, 3000) == Some(true) {
                    self.screen_check(ui, "100 % again");
                    self.finish(ui);
                }
            }
            _ => {}
        }
    }

    fn t_windows(&mut self, ui: &mut Ui) {
        match self.pc {
            0 => {
                if self.wait_on(ui, 15_000) == Some(true) {
                    self.second.set(1);
                    self.next();
                }
            }
            1 => {
                let o = ui.gpu_output(1);
                if o.on {
                    self.ok(o.adapter == ui.gpu_output(0).adapter, "the second window on the same device");
                    self.ok(ui.gpu_output(0).on, "the first window stays on its swapchain");
                    // a popup that does not fit: a window of its own on the CPU path
                    ui.popup_open(901);
                    self.next();
                } else if self.after(5000) {
                    self.ok(false, "the second window did not reach its swapchain");
                    self.finish(ui);
                }
            }
            2 => {
                if self.after(120) {
                    let r = plat::win_test(plat::WT_POPUP_RECT, 0, [0; 4]);
                    self.ok(r != -1, "a popup window is shown");
                    self.ok(ui.gpu_output(0).on, "the owner stays on its swapchain");
                    self.mode = 2; // closes the popup
                    self.next();
                }
            }
            3 => {
                if self.after(100) {
                    self.mode = 0;
                    self.second.set(0);
                    self.next();
                }
            }
            4 => {
                if self.after(150) {
                    self.ok(ui.gpu_output(0).on, "the second window closed: the first window goes on");
                    self.patch = 0xFFE0_E000;
                    self.next();
                }
            }
            5 => {
                if self.after(100) {
                    self.screen_check(ui, "the first window");
                    self.second.set(1);
                    self.next();
                }
            }
            6 => {
                if self.after(600) {
                    self.ok(ui.gpu_output(1).on, "the second window shows again, on its swapchain");
                    self.finish(ui);
                }
            }
            _ => {}
        }
    }

    fn t_cpu_only(&mut self, ui: &mut Ui, never: bool) {
        match self.pc {
            0 => {
                if self.after(if never { 800 } else { 300 }) {
                    let o = ui.gpu_output(0);
                    self.ok(!o.on && !o.building, "the CPU path");
                    self.ok(o.never == never, &format!("no output for this run: {} ({:?})", o.never, o.error));
                    self.ok(o.frames == 0, "nothing through a swapchain");
                    let p = plat::win_test(plat::WT_PIXEL, 0, [150, 120, 0, 0]);
                    self.ok(p == bgr(self.patch) as i64, &format!("the window's DC shows the frame ({p:06x})"));
                    self.patch = 0xFF00_90F0;
                    self.next();
                }
            }
            1 => {
                if self.after(100) {
                    let p = plat::win_test(plat::WT_PIXEL, 0, [150, 120, 0, 0]);
                    self.ok(p == bgr(self.patch) as i64, "a change shows");
                    self.screen_check(ui, "a change");
                    self.finish(ui);
                }
            }
            _ => {}
        }
    }

    fn t_lose(&mut self, ui: &mut Ui) {
        match self.pc {
            0 => {
                if self.wait_on(ui, 15_000) == Some(true) {
                    self.patch = 0xFFFF_3020;
                    self.next();
                }
            }
            1 => {
                if self.after(400) {
                    self.screen_check(ui, "before");
                    ui.gpu_test_lose();
                    self.patch = 0xFF20_30FF;
                    self.next();
                }
            }
            2 => {
                let o = ui.gpu_output(0);
                if !o.on {
                    self.ok(o.leaves == 1, "lost: back on the CPU path with the next frame");
                    self.mark[0] = o.cpu_frames;
                    self.next();
                } else if self.after(1000) {
                    self.ok(false, "lost: still on the swapchain");
                    self.finish(ui);
                }
            }
            3 => {
                if self.after(100) {
                    self.screen_check(ui, "the frame after the loss (GDI shows again)");
                    self.patch = 0xFF20_FF30;
                    self.next();
                }
            }
            4 => {
                if self.after(100) {
                    self.screen_check(ui, "the CPU path goes on");
                    self.next();
                }
            }
            5 => {
                if self.wait_on(ui, 8000) == Some(true) {
                    self.ok(ui.gpu_output(0).switches == 2, "a new device, the swapchain again");
                    self.patch = 0xFF80_90A0;
                    self.next();
                }
            }
            6 => {
                if self.after(100) {
                    self.screen_check(ui, "after the recovery");
                    self.finish(ui);
                }
            }
            _ => {}
        }
    }

    fn t_csd(&mut self, ui: &mut Ui) {
        const WM_NCHITTEST: u32 = 0x0084;
        match self.pc {
            0 => {
                if self.wait_on(ui, 15_000) == Some(true) {
                    let Some(r) = plat::win_get_rect(0) else { return };
                    let lp = |x: i32, y: i32| ((x as u32 & 0xFFFF) | (y as u32) << 16) as i32 as isize;
                    let top = plat::win_test_send(0, WM_NCHITTEST, 0, lp(r[0] + 200, r[1] + 1));
                    let mid = plat::win_test_send(0, WM_NCHITTEST, 0, lp(r[0] + 200, r[1] + 200));
                    self.ok(top == 12, &format!("the resize band at the top: HTTOP ({top})"));
                    self.ok(mid == 1, &format!("the middle: HTCLIENT ({mid})"));
                    self.finds_window(200, 1, "the band's point finds the window, not the child");
                    self.screen_check(ui, "the library's frame");
                    plat::win_test(plat::WT_MOVE, 0, [r[0], r[1], 700, 500]);
                    self.next();
                }
            }
            1 => {
                if ui.gpu_output(0).leaves == 1 {
                    self.next();
                } else if self.after(3000) {
                    self.ok(false, "a new size: no new frame");
                    self.finish(ui);
                }
            }
            2 => {
                if self.wait_on(ui, 5000) == Some(true) {
                    self.ok(ui.gpu_output(0).switches == 2, "a new size: the swapchain again");
                    self.next();
                }
            }
            3 => {
                if self.after(100) {
                    self.screen_check(ui, "a new size");
                    self.finish(ui);
                }
            }
            _ => {}
        }
    }

    fn t_monitors(&mut self, ui: &mut Ui) {
        match self.pc {
            0 => {
                if self.wait_on(ui, 15_000) == Some(true) {
                    let ms = plat::monitors();
                    let here = plat::win_get_rect(0).unwrap_or([0; 4]);
                    let inside = |m: &plat::Monitor| here[0] >= m.rect[0] && here[0] < m.rect[0] + m.rect[2] && here[1] >= m.rect[1] && here[1] < m.rect[1] + m.rect[3];
                    let Some(other) = ms.iter().find(|m| !inside(m)) else {
                        out("SKIP: one monitor");
                        self.done = true;
                        ui.quit();
                        return;
                    };
                    plat::win_set_rect(0, other.rect[0] + 60, other.rect[1] + 60, plat::PW_KEEP, plat::PW_KEEP);
                    self.next();
                }
            }
            1 => {
                if self.after(300) && self.wait_on(ui, 3000) == Some(true) {
                    self.screen_check(ui, "on the other monitor");
                    self.patch = 0xFFA0_60F0;
                    self.next();
                }
            }
            2 => {
                if self.after(100) {
                    self.screen_check(ui, "a change on the other monitor");
                    self.finish(ui);
                }
            }
            _ => {}
        }
    }

    fn t_idle(&mut self, ui: &mut Ui) {
        match self.pc {
            0 => {
                if self.wait_on(ui, 15_000) == Some(true) {
                    self.next();
                }
            }
            1 => {
                if self.after(300) {
                    let o = ui.gpu_output(0);
                    self.mark[0] = o.frames;
                    self.mark[1] = o.cpu_frames;
                    self.mark[2] = plat::win_test_usage().0;
                    self.next();
                }
            }
            2 => {
                if self.after(1000) {
                    let o = ui.gpu_output(0);
                    let cpu = plat::win_test_usage().0 - self.mark[2];
                    self.ok(o.frames == self.mark[0] && o.cpu_frames == self.mark[1], "1 s with nothing to do: no frame");
                    out(&format!("  CPU time over 1 s: {:.2} ms (the test's own timer included)", cpu as f64 / 1000.0));
                    self.ok(!o.pending, "no frame waits");
                    self.caret = true; // a text box with the focus: its caret blinks
                    self.mark[0] = o.frames;
                    self.mark[1] = o.upload_bytes;
                    self.next();
                }
            }
            3 => {
                if self.after(1700) {
                    let o = ui.gpu_output(0);
                    let (n, up) = (o.frames - self.mark[0], o.upload_bytes - self.mark[1]);
                    let each = up.checked_div(n).unwrap_or(0);
                    out(&format!("  caret over 1.7 s: {n} frames, {each} bytes each"));
                    self.ok((1..=8).contains(&n), "the caret blinks");
                    // the first frame includes the focus ring and the whole text box
                    self.ok(each < 256 << 10, "a blink is a small upload");
                    self.finish(ui);
                }
            }
            _ => {}
        }
    }
}

impl App for Test {
    fn frame(&mut self, ui: &mut Ui) {
        if ui.key(VK_F9) {
            self.patch = self.patch ^ 0x00FF_FFFF;
            if let Some(b) = &self.bench {
                if b.full {
                    self.full = if self.full == 0xFF20_4060 { 0xFF60_4020 } else { 0xFF20_4060 };
                }
                if b.burst {
                    self.mark[3] += 1;
                    self.patch = 0xFF00_0000 | (self.mark[3] as u32 & 0xFFFF);
                }
            }
        }
        // the picture: six bands, a patch that changes, or one colour over everything
        let (w, h) = (ui.fb().w, ui.fb().h);
        if self.full != 0 {
            ui.draw_color(self.full);
            ui.draw_rect(0, 0, w, h);
        } else {
            for k in 0..6 {
                ui.draw_color(BANDS[k as usize]);
                // the same split as band_of
                let x0 = (k * w + 5) / 6;
                let x1 = ((k + 1) * w + 5) / 6;
                ui.draw_rect(x0, 0, x1 - x0, h);
            }
            ui.draw_color(self.patch);
            ui.draw_rect(100, 100, 200, 40);
        }
        if self.caret {
            ui.next_size(200, 30);
            if !self.t_first {
                self.t_first = true;
                ui.focus(77);
            }
            let mut t = core::mem::take(&mut self.text);
            ui.textbox(77, None, &mut t, 0);
            self.text = t;
        }
        let second = self.second.clone();
        let mut close = false;
        if ui.window_begin(900, "zweites Fenster", Some(&second), WF_TOOL | WF_RESIZABLE, None) {
            ui.draw_color(0xFF60_4080);
            ui.draw_rect(0, 0, 400, 300);
            ui.window_end();
        } else {
            ui.window_end();
        }
        if ui.flyout_begin(901, 0) {
            ui.text(TS_BODY, "Popup");
            if self.mode == 2 {
                close = true;
            }
        }
        ui.flyout_end();
        if close {
            ui.popup_close();
        }
        if self.done {
            return;
        }
        if self.bench.is_none() {
            if !self.started {
                self.started = true; // the waits of the tests count from the first pass
                self.since = plat::time_us();
            }
            self.passes += 1;
            let name = core::mem::take(&mut self.name);
            match name.as_str() {
                "switch" => self.t_switch(ui),
                "resize" => self.t_resize(ui),
                "minimize" => self.t_minimize(ui),
                "dpi" => self.t_dpi(ui),
                "windows" => self.t_windows(ui),
                "off" => self.t_cpu_only(ui, false),
                "noloader" => self.t_cpu_only(ui, true),
                "lose" => self.t_lose(ui),
                "idle" => self.t_idle(ui),
                "csd" => self.t_csd(ui),
                "monitors" => self.t_monitors(ui),
                _ => {
                    self.done = true;
                    ui.quit();
                }
            }
            self.name = name;
            // the next look (a real timer: the window loop goes on as it does for the app)
            ui.timer(1, 5);
        }
    }

    fn command(&mut self, ui: &mut Ui, id: u32, _arg: u32) -> bool {
        if id == UIC_TIMER {
            ui.invalidate();
            return true;
        }
        if id == UIC_EXIT && self.bench.is_some() {
            let o = ui.gpu_output(0);
            out(&format!("frames {} cpu_frames {} upload_bytes {} reserved {} on {}", o.frames, o.cpu_frames, o.upload_bytes, o.reserved, o.on));
        }
        false
    }
}

// ---------------------------------------------------------------- latency

struct Bench {
    full: bool,
    burst: bool,
    stop: AtomicBool,
    hwnd: AtomicU64,
    x: AtomicU32,
    y: AtomicU32,
}

/// the thread: a key press, the present, the screen; `burst`: the next press as soon as the
/// present returned
fn bench_thread(b: Arc<Bench>, n: usize, burst: bool) {
    while b.hwnd.load(Ordering::Relaxed) == 0 {
        plat::sleep_ms(5);
    }
    plat::sleep_ms(if plat::env("jfgui_GPU").as_deref() != Some("0") { 2500 } else { 500 });
    let hwnd = b.hwnd.load(Ordering::Relaxed) as isize;
    let (x, y) = (b.x.load(Ordering::Relaxed) as i32, b.y.load(Ordering::Relaxed) as i32);
    let (mut to_present, mut to_screen) = (Vec::new(), Vec::new());
    let mut seed = 12345u32;
    let t00 = plat::time_us();
    if burst {
        // presses as fast as the UI takes them; the patch's colour is the press's number: how old is
        // what the screen shows?
        let issued: Arc<Vec<AtomicU64>> = Arc::new((0..n + 2).map(|_| AtomicU64::new(0)).collect());
        let done = Arc::new(AtomicBool::new(false));
        let (i2, d2) = (issued.clone(), done.clone());
        let lags = Arc::new(rt::sync::SpinLock::new(Vec::new()));
        let l2 = lags.clone();
        let th = plat::thread_spawn(0, Box::new(move || {
            let mut last = 0;
            while !d2.load(Ordering::Relaxed) {
                let c = screen_px(x, y) as usize;
                if c != last && c < i2.len() {
                    last = c;
                    let t = i2[c].load(Ordering::Relaxed);
                    if t != 0 {
                        l2.lock().push(plat::time_us().saturating_sub(t));
                    }
                }
            }
        }));
        for k in 0..n {
            let frames = plat::present_time().0;
            let t0 = plat::time_us();
            issued[k + 1].store(t0, Ordering::Relaxed);
            plat::win_test_post(hwnd, WM_KEYDOWN, VK_F9 as usize, 0x0001_0001);
            while plat::time_us() < t0 + 500_000 && plat::present_time().0 == frames {
                plat::thread_yield();
            }
            to_present.push(plat::time_us() - t0);
            plat::win_test_post(hwnd, WM_KEYUP, VK_F9 as usize, 0xC001_0001u32 as isize);
        }
        done.store(true, Ordering::Relaxed);
        if let Some(t) = th {
            t.join();
        }
        to_screen = core::mem::take(&mut *lags.lock());
    }
    for _ in 0..(if burst { 0 } else { n }) {
        let before = screen_px(x, y);
        let frames = plat::present_time().0;
        let t0 = plat::time_us();
        plat::win_test_post(hwnd, WM_KEYDOWN, VK_F9 as usize, 0x0001_0001);
        // the present returned
        let mut t1 = t0;
        while plat::time_us() < t0 + 500_000 {
            if plat::present_time().0 != frames {
                t1 = plat::time_us();
                break;
            }
            plat::thread_yield();
        }
        to_present.push(t1 - t0);
        plat::win_test_post(hwnd, WM_KEYUP, VK_F9 as usize, 0xC001_0001u32 as isize);
        // the screen changed
        let mut t2 = t0;
        while plat::time_us() < t0 + 500_000 {
            if screen_px(x, y) != before {
                t2 = plat::time_us();
                break;
            }
        }
        to_screen.push(t2.saturating_sub(t0));
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        plat::sleep_ms(15 + (seed >> 28));
    }
    let total = plat::time_us() - t00;
    let stat = |v: &mut Vec<u64>| -> (f64, f64, f64) {
        if v.is_empty() {
            return (0.0, 0.0, 0.0);
        }
        v.sort_unstable();
        (v[v.len() / 2] as f64 / 1000.0, v[v.len() * 95 / 100] as f64 / 1000.0, v[v.len() - 1] as f64 / 1000.0)
    };
    let (pm, pp, px) = stat(&mut to_present);
    let (sm, sp, sx) = stat(&mut to_screen);
    plat::write_stdout(format!(
        "RESULT n={n} present_median={pm:.3} present_p95={pp:.3} present_max={px:.3} screen_median={sm:.2} screen_p95={sp:.2} screen_max={sx:.2} total_ms={:.0}\n",
        total as f64 / 1000.0).as_bytes());
    b.stop.store(true, Ordering::Relaxed);
}

struct LatencyApp {
    t: Test,
    b: Arc<Bench>,
}

impl App for LatencyApp {
    fn frame(&mut self, ui: &mut Ui) {
        self.t.frame(ui);
        if self.b.hwnd.load(Ordering::Relaxed) == 0 {
            if let (Some(n), Some(r)) = (plat::win_native(0), plat::win_get_rect(0)) {
                self.b.x.store((r[0] + 150) as u32, Ordering::Relaxed);
                self.b.y.store((r[1] + 120) as u32, Ordering::Relaxed);
                self.b.hwnd.store(n.hwnd as u64, Ordering::Relaxed);
            }
        }
        if self.b.stop.load(Ordering::Relaxed) {
            let o = ui.gpu_output(0);
            plat::write_stdout(format!("path: on={} frames={} cpu_frames={} mode={} adapter={}\n", o.on, o.frames, o.cpu_frames, o.present_mode, o.adapter).as_bytes());
            ui.quit();
        }
        ui.timer(1, 50);
    }
    fn command(&mut self, ui: &mut Ui, id: u32, arg: u32) -> bool {
        self.t.command(ui, id, arg)
    }
}

fn new_test(name: &str) -> Test {
    Test {
        name: String::from(name),
        fails: 0,
        pc: 0,
        since: plat::time_us(),
        patch: 0xFFFF_FF00,
        full: 0,
        done: false,
        t_first: false,
        started: false,
        passes: 0,
        since_pass: 0,
        watch: None,
        mark: [0; 4],
        second: Shared::new(0),
        text: String::new(),
        caret: false,
        mode: 0,
        bench: None,
    }
}

// ---------------------------------------------------------------- without a window

fn compile_shaders() -> Option<(Vec<u32>, Vec<u32>)> {
    let opt = wgsl::CompileOptions { robust: false, ..wgsl::CompileOptions::new(shader::BASIS) };
    match wgsl::compile(ui::GPU_OUT_WGSL, &opt) {
        Ok(o) => Some((o.entry("vs")?.words.clone(), o.entry("fs")?.words.clone())),
        Err(d) => {
            out(&format!("FAIL gpu_out.wgsl: {:?}", d.first().map(|x| format!("{x}"))));
            None
        }
    }
}

fn gen_shaders(path: &str) -> i32 {
    let Some((vs, fs)) = compile_shaders() else { return 1 };
    let mut s = String::from("// written by uigputest --gen-shaders from gpu_out.wgsl (shader::BASIS, no robustness); do not edit\n");
    for (name, w) in [("VS", &vs), ("FS", &fs)] {
        s.push_str(&format!("pub const {name}: &[u32] = &[\n"));
        for row in w.chunks(10) {
            s.push_str("    ");
            for v in row {
                s.push_str(&format!("{v:#x}, "));
            }
            s.truncate(s.len() - 1);
            s.push('\n');
        }
        s.push_str("];\n");
    }
    if plat::write_file(path, s.as_bytes()) {
        out(&format!("wrote {path}: {} + {} words", vs.len(), fs.len()));
        0
    } else {
        1
    }
}

fn t_shaders() -> i32 {
    let Some((vs, fs)) = compile_shaders() else { return 1 };
    let mut fails = 0;
    for (name, a, b) in [("vs", &vs[..], ui::GPU_OUT_VS), ("fs", &fs[..], ui::GPU_OUT_FS)] {
        if a == b {
            out(&format!("  ok   {name}: {} words as wgsl::compile makes them", b.len()));
        } else {
            out(&format!("FAIL {name}: the embedded SPIR-V ({} words) is not what wgsl::compile makes ({} words): uigputest --gen-shaders lib/ui/gpu_out_spv.rs", b.len(), a.len()));
            fails += 1;
        }
    }
    (fails != 0) as i32
}

fn t_adapter() -> i32 {
    use gpu::AdapterKind::*;
    let mut fails = 0;
    let mut ok = |c: bool, what: &str| {
        if c {
            out(&format!("  ok   {what}"));
        } else {
            out(&format!("FAIL {what}"));
            fails += 1;
        }
    };
    let r = gpu::low_power_rank;
    ok(r(Integrated, true) < r(Discrete, true), "both drive the display: integrated first");
    ok(r(Discrete, true) < r(Integrated, false), "the adapter that drives the display before one that does not");
    ok(r(Integrated, false) < r(Discrete, false), "neither drives it: integrated first");
    ok(r(Cpu, false).is_none() && r(Virtual, true).is_none() && r(Other, true).is_none(), "CPU, virtual and other adapters: never");
    let w = plat::win_create("uigputest", 200, 150, plat::PWF_SYSFRAME, plat::PW_NOOWNER, 40, 40);
    let display = plat::win_display_adapter(w);
    out(&format!("  the display's adapter: {display:04x?}"));
    match gpu::Instance::new() {
        Err(e) => out(&format!("SKIP: {e}")),
        Ok(inst) => match inst.adapters_low_power(display) {
            Err(e) => out(&format!("SKIP: {e}")),
            Ok(list) => {
                for a in &list {
                    let i = a.info();
                    out(&format!("  {} ({:?}, {:04x}:{:04x})", i.name, i.kind, i.vendor_id, i.device_id));
                }
                let first = list[0].info();
                if let Some((v, d)) = display {
                    ok(first.vendor_id == v && first.device_id == d, "the display's adapter comes first");
                }
                let all = inst.adapter().map(|a| a.info().name.clone()).unwrap_or_default();
                out(&format!("  instance.adapter() keeps its choice: {all}"));
            }
        },
    }
    plat::win_destroy(w);
    (fails != 0) as i32
}

/// what the screen does with GDI and a swapchain on one window, and with a child that holds the
/// swapchain (printed: it may differ between drivers)
fn t_hwnd() -> i32 {
    use gpu::*;
    let pump = |ms: u64| {
        let dl = plat::time_us() + ms * 1000;
        loop {
            plat::wait(dl);
            while plat::evq_pop().is_some() {}
            if plat::time_us() >= dl {
                break;
            }
        }
    };
    let inst = match Instance::new() {
        Ok(i) => i,
        Err(e) => {
            out(&format!("SKIP: {e}"));
            return 0;
        }
    };
    let (Ok(adapter), true) = (inst.adapter(), true) else {
        out("SKIP: no adapter");
        return 0;
    };
    let Ok(dev) = adapter.create_device(&DeviceDesc { max_level: Level::Basis }) else {
        out("SKIP: no device");
        return 0;
    };
    let w = plat::win_create("uigputest", 300, 200, plat::PWF_SYSFRAME, plat::PW_NOOWNER, 60, 60);
    plat::win_test(plat::WT_NOANIM, w, [0; 4]);
    plat::win_show(w, plat::PWS_SHOWNA);
    pump(100);
    let Some(r) = plat::win_get_rect(w) else { return 1 };
    let (cw, ch) = plat::win_size(w);
    let gdi = |c: u32| {
        let (cw, ch) = plat::win_size(w);
        let px: Vec<u8> = (0..cw * ch).flat_map(|_| [c as u8, (c >> 8) as u8, (c >> 16) as u8, 255]).collect();
        plat::win_present(w, &plat::Frame { px: &px, w: cw, h: ch, stride: cw * 4 }, None);
    };
    let gpu_frame = |sc: &mut Swapchain, c: [f32; 4]| -> gpu::Result<()> {
        let Some(fr) = sc.acquire()? else { return Ok(()) };
        let sc = &*sc;
        let mut e = dev.encoder()?;
        e.begin_render(&RenderPassDesc {
            color: &[ColorAttachment { view: sc.view(&fr), load: Load::Clear, clear: c, store: true, before: Layout::Undefined, after: Layout::Present }],
            depth: None,
        });
        e.end_render();
        let cmd = e.finish()?;
        sc.present(fr, [cmd]).map(|_| ())
    };
    let seen = || screen_px(r[0] + 50, r[1] + 50);
    if plat::win_test(plat::WT_FROM_POINT, w, [50, 50, 0, 0]) != 1 {
        out("note: another window lies on top of ours: the colours below say nothing");
    }
    gdi(0xFF0000);
    pump(80);
    out(&format!("  GDI red: the screen shows {:06x}", seen()));
    {
        let Ok(surf) = Surface::new(&inst, w) else { return 1 };
        let Ok(mut sc) = dev.create_swapchain(&surf, &SwapchainDesc::DEFAULT) else { return 1 };
        pump(80);
        out(&format!("  a swapchain on the window, nothing presented: {:06x}", seen()));
        let _ = gpu_frame(&mut sc, [0.0, 1.0, 0.0, 1.0]);
        pump(80);
        out(&format!("  green presented: {:06x}", seen()));
        gdi(0x0000FF);
        pump(80);
        out(&format!("  GDI blue on top of it: {:06x}", seen()));
    }
    pump(80);
    out(&format!("  the swapchain and the surface gone: {:06x}", seen()));
    gdi(0xFFFFFF);
    pump(100);
    let back = seen();
    out(&format!("  GDI white afterwards: {back:06x}: GDI {} on this window", if back == 0xFFFFFF { "shows again" } else { "stays invisible" }));
    plat::win_test(plat::WT_REDRAW, w, [0; 4]);
    pump(80);
    out(&format!("  after a redraw: {:06x}", seen()));
    plat::win_set_rect(w, plat::PW_KEEP, plat::PW_KEEP, cw + 1, ch);
    pump(80);
    gdi(0xFFFFFF);
    pump(80);
    out(&format!("  after a new size: {:06x}", seen()));
    plat::win_show(w, plat::PWS_HIDE);
    pump(80);
    plat::win_show(w, plat::PWS_SHOWNA);
    pump(80);
    out(&format!("  after hiding and showing it: {:06x}", seen()));
    plat::win_destroy(w);
    // the same with a child that holds the swapchain
    let w = plat::win_create("uigputest", 300, 200, plat::PWF_SYSFRAME, plat::PW_NOOWNER, 60, 60);
    plat::win_test(plat::WT_NOANIM, w, [0; 4]);
    plat::win_show(w, plat::PWS_SHOWNA);
    pump(100);
    let Some(r) = plat::win_get_rect(w) else { return 1 };
    let seen = || screen_px(r[0] + 50, r[1] + 50);
    let gdi = |c: u32| {
        let px: Vec<u8> = (0..cw * ch).flat_map(|_| [c as u8, (c >> 8) as u8, (c >> 16) as u8, 255]).collect();
        plat::win_present(w, &plat::Frame { px: &px, w: cw, h: ch, stride: cw * 4 }, None);
    };
    gdi(0xFF0000);
    pump(80);
    let Some(nw) = plat::win_gpu_child(w) else { return 1 };
    let Ok(surf) = Surface::native(&inst, nw, w) else { return 1 };
    let Ok(mut sc) = dev.create_swapchain(&surf, &SwapchainDesc::DEFAULT) else { return 1 };
    let _ = gpu_frame(&mut sc, [0.0, 1.0, 0.0, 1.0]);
    plat::win_gpu_child_show(w, true);
    pump(100);
    out(&format!("  the child with a swapchain, green: {:06x}", seen()));
    gdi(0x0000FF);
    plat::win_gpu_child_show(w, false);
    pump(100);
    out(&format!("  GDI blue, the child hidden: {:06x}", seen()));
    drop(sc);
    drop(surf);
    plat::win_destroy(w);
    pump(20);
    0
}

fn run() -> i32 {
    let args = plat::args();
    let val = |n: &str| plat::arg_value(n);
    if let Some(p) = val("--gen-shaders") {
        return gen_shaders(&p);
    }
    let plain = args.iter().any(|a| a == "--plain");
    if let Some(b) = val("--bench") {
        plat::setenv("jfgui_STATE", Some("0"));
        plat::setenv("jfgui_TEST_NOACTIVATE", Some("1"));
        if b == "latency" {
            let full = val("--scene").as_deref() == Some("full");
            let n: usize = val("--n").and_then(|v| v.parse().ok()).unwrap_or(200);
            let burst = args.iter().any(|a| a == "--burst");
            let bench = Arc::new(Bench { full, burst, stop: AtomicBool::new(false), hwnd: AtomicU64::new(0), x: AtomicU32::new(0), y: AtomicU32::new(0) });
            let mut t = new_test("bench");
            t.bench = Some(bench.clone());
            let b2 = bench.clone();
            let th = plat::thread_spawn(0, Box::new(move || bench_thread(b2, n, burst)));
            drop(th);
            let mut app = LatencyApp { t, b: bench };
            let d = AppDesc { w: val("--w").and_then(|v| v.parse().ok()).unwrap_or(1280), h: val("--h").and_then(|v| v.parse().ok()).unwrap_or(800), ..DESC };
            return ui::run_window(&d, &mut app, &style::STYLE);
        }
        if b == "idle" || b == "caret" {
            let mut t = new_test("bench");
            t.bench = Some(Arc::new(Bench { full: false, burst: false, stop: AtomicBool::new(false), hwnd: AtomicU64::new(0), x: AtomicU32::new(0), y: AtomicU32::new(0) }));
            t.caret = b == "caret";
            let d = AppDesc { w: 1280, h: 800, ..DESC };
            return ui::run_window(&d, &mut t, &style::STYLE);
        }
        out("--bench latency|idle|caret");
        return 2;
    }
    let Some(test) = val("--test") else {
        plat::write_stderr(b"uigputest: --test NAME, --gen-shaders FILE, --bench latency (see apps/uigputest/main.rs)\n");
        return 2;
    };
    out(&format!("== {test}"));
    match test.as_str() {
        "shaders" => return t_shaders(),
        "adapter" => return t_adapter(),
        "hwnd" => return t_hwnd(),
        _ => {}
    }
    plat::setenv("jfgui_STATE", Some("0"));
    plat::setenv("jfgui_TEST_NOACTIVATE", Some("1"));
    // `switch` without the variable: the swapchain is the default
    plat::setenv("jfgui_GPU", if test == "off" { Some("0") } else if test == "switch" { None } else { Some("1") });
    if !plain {
        plat::setenv("jfgui_GPU_VERIFY", Some("1"));
        plat::setenv("jfgui_GPU_VALIDATE", Some("1"));
    }
    match test.as_str() {
        "noloader" => {
            plat::setenv("jfgui_GPU_DLL", Some("missing-vulkan-loader.dll"));
        }
        "lose" => {
            plat::setenv("jfgui_GPU_RETRY_MS", Some("300"));
        }
        _ => {}
    }
    let mut t = new_test(&test);
    let d = if test == "csd" { AppDesc { flags: AF_RESIZABLE | AF_CUSTOM_TITLEBAR, ..DESC } } else { DESC };
    let code = ui::run_window(&d, &mut t, &style::STYLE);
    if code != 0 {
        return code;
    }
    if t.fails != 0 {
        out(&format!("{} check(s) failed", t.fails));
        return 1;
    }
    if !t.done {
        out("FAIL the test did not end");
        return 1;
    }
    out("all checks passed");
    0
}

ui::main!(run());
