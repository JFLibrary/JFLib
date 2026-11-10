// "3D-Ansicht" (G5e2): a render3d scene between the widgets (ui.gpu_view, docs/GPU.md "3D-Ansicht"): five shapes on a
// floor under the sun, turned by a finger or by itself, a panel that can lie over it like a popup.  Its colours are
// sRGB codes: render3d encodes them for the view's 8-bit target (RendererDesc::encode_srgb).  Without a GPU (a
// static Linux program, jfgui_GPU=0) the content paints a grey rectangle.
//
// "Prüfen" compares the view in the framebuffer with the same frame drawn offscreen by the same renderer on the same
// device and with render3dtest's ray caster (cpu.rs, common.rs, gltfgen.rs, included as they are), and says so on
// the page and in the log (lines "viewcheck: ..."); in the browser the read-backs come later and the check waits for
// them.  apps/androidGallery/tests/view_web.mjs (browser) and android_gpu.py --phases view (emulator, phone) go
// through it.  The app keeps its device as it was (no AF_GPU_3D): the view draws on the UI's device at level Basis.
//
// jfgui_GPU_VIEWBENCH=N (measurements, the counterpart of uigputest --bench view in the browser and on Android): a view
// of render3dtest's gallery (3000 instances) 700 px high, turning as fast as the loop takes it; N frames after 3 s, per
// frame the interval, render and present of the output, the view's draw (record and submit), presented frames; one
// line "viewbench: ...".

use alloc::format;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::cell::RefCell;

use gpu::{ColorAttachment, CommandEncoder, Device, GpuError, Load, SubmitId};
use render3d::{Camera, Material, Renderer, RendererDesc, Scene, Sun};
use ui::*;
use ui::i18n::t;

use crate::common::{ball, cube, gallery_fill, gallery_specs, plate, quat, Geo, Off, Spec, CLEAR, GALLERY_N};
use crate::cpu::{self, RefCamera, RefScene};

const ID_VIEW: u32 = 9100;
const ID_TURN: u32 = 9101;
const ID_OVER: u32 = 9102;
const ID_CHECK: u32 = 9103;
const ID_LOSE: u32 = 9104;
/// the page's timer: the animation and the check's steps
pub const TIMER: u32 = 4300;
/// what the content paints without a device
const PAINT: u32 = 0xFF55_2266;

/// the scene: render3dtest's shapes, as uigputest's views have it (apps/uigputest/view.rs world())
fn world() -> (Vec<Geo>, Vec<[f32; 3]>, Vec<Spec>) {
    let floor = quat([1.0, 0.0, 0.0], -core::f64::consts::FRAC_PI_2);
    let none = [0.0f32, 0.0, 0.0, 1.0];
    let spin = quat(cpu::norm([1.0, 2.0, 0.5]), 0.9);
    let at = |mesh: u32, material: u32, pos: [f64; 3], rot: [f32; 4], scale: [f32; 3]| Spec { mesh, material, pos, rot, scale };
    (vec![cube(), ball(12, 18), plate()],
     vec![[1.0, 0.5, 0.25], [0.25, 1.0, 0.5], [0.5, 0.25, 1.0], [0.85, 0.85, 0.8]],
     vec![at(2, 3, [0.0, -1.0, 0.0], floor, [6.0, 6.0, 1.0]), at(0, 0, [-2.2, 0.0, 0.0], spin, [1.0; 3]), at(1, 1, [2.2, 0.0, 0.5], none, [1.2; 3]),
          at(0, 2, [0.0, -0.4, -2.5], none, [0.6; 3]), at(1, 0, [0.4, 0.1, 2.2], none, [0.5, 1.1, 0.5])])
}

fn sun() -> Sun {
    Sun { direction: [0.3, 0.8, 0.5], color: [0.7 * core::f32::consts::PI; 3], ambient: [0.3; 3] }
}

fn orbit(gallery: bool, a: f64, w: u32, h: u32) -> Camera {
    let (s, c) = cpu::sincos(a);
    let aspect = w as f64 / h.max(1) as f64;
    if gallery {
        // (uigputest view.rs orbit, Kind::Gallery)
        return Camera::look_at([90.0 * s, 12.0, 90.0 * c - 15.0], [0.0, 0.0, -15.0], [0.0, 1.0, 0.0], 1.0, aspect, 0.1, f64::INFINITY);
    }
    Camera::look_at([7.5 * s, 4.0, 7.5 * c], [0.0, 0.0, 0.0], [0.0, 1.0, 0.0], 0.9, aspect, 0.1, f64::INFINITY)
}

/// render3d in the view: a Renderer and a Scene on the device lib/ui hands over
#[derive(Default)]
pub struct Content {
    angle: f64,
    /// render3dtest's gallery (jfgui_GPU_VIEWBENCH) instead of the five shapes
    gallery: bool,
    dev: Option<Device>,
    r: Option<Renderer>,
    s: Option<Scene>,
    /// the camera and size of the last frame
    cam: Option<Camera>,
    size: (u32, u32),
    attaches: u32,
    detaches: u32,
}

impl GpuContent for Content {
    fn attach(&mut self, dev: &Device) -> gpu::Result<()> {
        let mut r = Renderer::new(dev, &RendererDesc::new(GPU_VIEW_FORMAT))?;
        let mut s;
        if self.gallery {
            let (meshes, materials) = gallery_fill(&mut r)?;
            s = Scene::new(&r)?;
            for spec in gallery_specs(meshes, materials, GALLERY_N, [0.0; 3]) {
                s.add(spec.instance())?;
            }
        } else {
            let (geos, colors, specs) = world();
            for g in &geos {
                r.add_mesh(&g.data())?;
            }
            for c in &colors {
                r.add_material(&Material::color(c[0], c[1], c[2]))?;
            }
            s = Scene::new(&r)?;
            for spec in &specs {
                s.add(spec.instance())?;
            }
        }
        s.set_sun(&sun());
        (self.r, self.s, self.dev) = (Some(r), Some(s), Some(dev.clone()));
        self.attaches += 1;
        Ok(())
    }

    fn detach(&mut self) {
        (self.s, self.r, self.dev) = (None, None, None);
        self.detaches += 1;
    }

    fn draw<'a>(&'a mut self, e: &mut CommandEncoder<'a>, target: ColorAttachment<'a>, size: (u32, u32)) -> gpu::Result<()> {
        let cam = orbit(self.gallery, self.angle, size.0, size.1);
        {
            let (Some(r), Some(s)) = (self.r.as_mut(), self.s.as_mut()) else { return Err(GpuError::Invalid("not attached")) };
            r.set_size(size.0, size.1)?;
            s.set_camera(&cam);
            s.prepare(r)?;
        }
        (self.cam, self.size) = (Some(cam), size);
        let me: &'a Content = self;
        let (Some(r), Some(s)) = (me.r.as_ref(), me.s.as_ref()) else { return Err(GpuError::Invalid("not attached")) };
        r.record(e, s, ColorAttachment { load: Load::Clear, clear: CLEAR, ..target }, None)
    }

    fn submitted(&mut self, id: SubmitId) {
        if let Some(s) = self.s.as_mut() {
            s.submitted(id);
        }
    }

    fn paint(&self, g: &mut Gfx, r: [i32; 4]) {
        g.dc.set_color(PAINT);
        g.dc.fill(g.sf, r[0], r[1], r[2], r[3]);
    }
}

/// a check on its way: the frame offscreen (in the browser its read-back comes later), the view's rectangle, from
/// which row on it is compared (what lies over the rows above), against the rays too
struct Check {
    what: &'static str,
    off: Off,
    rect: [i32; 4],
    from: i32,
    ray: bool,
    cam: Camera,
    frame: u64,
    started: u64,
    /// the offscreen frame's submission (Vulkan: waited for; the browser: its read-back says when)
    id: SubmitId,
    /// settled by hand (Ui::gpu_settle) or not; when the view's CPU copy had the frame (0: not yet)
    settle: bool,
    ready_at: u64,
}

/// (self-tests) jfgui_GPU_VIEWCHECK=1: the page goes through these steps by itself and says what it finds in the log
/// (view_web.mjs, android_gpu.py --phases view; the name begins with jfgui_GPU_ so the browser's ?gpuenv= sets it).
/// A hold is a pause in which the test looks at the screen from outside
#[derive(Clone, Copy, PartialEq, Debug)]
enum Step {
    /// a device and a frame of the view, or a verdict without one
    Start,
    Check { what: &'static str, from: i32, settle: bool, ray: bool },
    Hold(&'static str),
    Turn(u32),
    Rest(u64),
    Over(bool),
    Lose,
    /// a new device after the loss: a frame of the view on it, then a check
    Again,
    Done,
}

const STEPS: [Step; 15] = [Step::Start, Step::Check { what: "the first frame", from: 0, settle: true, ray: true }, Step::Hold("at rest"), Step::Turn(30),
                           Step::Rest(600), Step::Check { what: "at rest after turning, not settled by hand", from: 0, settle: false, ray: true }, Step::Over(true),
                           Step::Turn(20), Step::Hold("a popup over the view"), Step::Check { what: "below the popup", from: 130, settle: true, ray: false },
                           Step::Over(false), Step::Rest(400), Step::Check { what: "the popup closed, at rest", from: 0, settle: false, ray: true }, Step::Lose,
                           Step::Again];
/// how long a hold lasts, how long a step may wait (us)
const HOLD_US: u64 = 2_500_000;
const WAIT_US: u64 = 20_000_000;

pub struct View3d {
    content: Rc<RefCell<Content>>,
    ver: u32,
    turning: i32,
    over: i32,
    check: Option<Check>,
    /// what the last check found, how many there were and failed
    result: String,
    checks: u32,
    failed: u32,
    rect: [i32; 4],
    drag_x: Option<i32>,
    /// (jfgui_GPU_VIEWCHECK) the step, since when, a count it keeps; the generation before a loss
    step: Option<usize>,
    since: u64,
    left: u32,
    generation: u32,
    /// (jfgui_GPU_VIEWBENCH) frames to measure, the start, the counters at the first measured frame (presented, render
    /// us, present us, draw us, the view's frames), the intervals between frames of the view
    bench: u32,
    bench_t0: u64,
    bench_base: Option<[u64; 5]>,
    bench_gaps: Vec<u64>,
    bench_last: (u64, u64),
}

impl Default for View3d {
    fn default() -> Self {
        Self { content: Rc::new(RefCell::new(Content { angle: 0.3, ..Default::default() })), ver: 1, turning: 0, over: 0, check: None, result: String::new(),
               checks: 0, failed: 0, rect: [0; 4], drag_x: None, step: None, since: 0, left: 0, generation: 0, bench: 0, bench_t0: 0, bench_base: None,
               bench_gaps: Vec::new(), bench_last: (0, 0) }
    }
}

/// the rectangle r of the framebuffer, rows of B, G, R, x
fn fb_rect(fb: &gfx::Surface, r: [i32; 4]) -> Vec<u8> {
    let mut v = Vec::with_capacity((r[2] * r[3] * 4) as usize);
    for y in r[1]..r[1] + r[3] {
        let at = y as usize * fb.stride as usize + r[0] as usize * 4;
        v.extend_from_slice(&fb.px[at..at + r[2] as usize * 4]);
    }
    v
}

fn say(s: &str) {
    plat::write_stdout(format!("viewcheck: {s}\n").as_bytes());
}

impl View3d {
    pub fn page(&mut self, ui: &mut Ui) {
        if self.step.is_none() && self.left != u32::MAX && plat::env("jfgui_GPU_VIEWCHECK").is_some_and(|v| v == "1") {
            self.step = Some(0);
            self.since = plat::time_us();
            say("start");
            ui.timer(TIMER, 16);
        }
        if self.bench_t0 == 0 {
            if let Some(n) = plat::env("jfgui_GPU_VIEWBENCH").and_then(|v| v.parse::<u32>().ok()).filter(|n| *n > 0) {
                (self.bench, self.bench_t0) = (n, plat::time_us());
                self.content.borrow_mut().gallery = true;
                ui.timer(TIMER, 1);
            } else {
                self.bench_t0 = 1;
            }
        }
        ui.text(TS_BODY | TX_WRAP | TX_SECONDARY, t("Eine 3D-Szene zwischen den Widgets: Ziehen dreht sie, die GPU zeichnet sie in die Oberfläche."));
        ui.next_size(SIZE_FILL, if self.bench > 0 { 700 } else { 240 });
        let inp = ui.gpu_view(ID_VIEW, &self.content, self.ver).clone();
        self.rect = inp.rect;
        if inp.ev & EV_DRAG != 0 {
            if let Some(x) = self.drag_x.filter(|x| *x != inp.x) {
                self.content.borrow_mut().angle += (inp.x - x) as f64 * 0.01;
                self.ver = self.ver.wrapping_add(1);
            }
            self.drag_x = Some(inp.x);
        } else {
            self.drag_x = None;
        }
        // the panel over the view's corner, as a popup lies there (its own layer)
        if self.over != 0 {
            let r = self.rect;
            let prev = ui.cmd_layer(L_POPUP);
            ui.draw_color(0x6000_0000);
            ui.cmd_shadow(r[0] + 16, r[1] + 16, 180, 90, 8, 12);
            ui.cmd_rrect(r[0] + 16, r[1] + 16, 180, 90, 8, 1, 0xE0F0_E8D8, 0xFF40_4040, 0xFF20_2020);
            ui.draw_color(0xFF10_2030);
            ui.draw_text(r[0] + 30, r[1] + 48, t("über der Ansicht"), (14u32 << 16) | FONT_REG);
            ui.cmd_layer(prev);
        }
        ui.box_begin(0, BOX_H, 8);
        if ui.checkbox(ID_TURN, t("Drehen"), &mut self.turning) & EV_ACT != 0 && self.turning != 0 {
            ui.timer(TIMER, 16);
        }
        let _ = ui.checkbox(ID_OVER, t("Darüber"), &mut self.over);
        ui.box_end();
        ui.box_begin(0, BOX_H, 8);
        if ui.button(ID_CHECK, t("Prüfen")) & EV_ACT != 0 && self.check.is_none() {
            self.start_check(ui, "by hand", 0, true, true);
        }
        if ui.button(ID_LOSE, t("Gerät verlieren")) & EV_ACT != 0 {
            ui.gpu_test_lose();
            say("the device lost (Ui::gpu_test_lose)");
        }
        ui.box_end();
        let i = ui.gpu_view_info(0, ID_VIEW).unwrap_or_default();
        let o = ui.gpu_output(0);
        let dev = if i.device {
            format!("{} ({:?}{})", if i.output { o.adapter.as_str() } else { t("eigenes Gerät") }, i.level.unwrap_or(gpu::Level::Basis),
                    if i.apart { t(", von der Ausgabe gezeichnet") } else { "" })
        } else {
            String::from(t("kein Gerät"))
        };
        ui.text(TS_CAPTION | TX_WRAP | TX_SECONDARY, &format!("{dev}; {} {}, {} {}", i.frames, t("Bilder"), i.reads, t("zurückgelesen")));
        if !self.result.is_empty() {
            ui.text(TS_BODY | TX_WRAP, &self.result);
        }
    }

    /// the frame the view shows offscreen too, its read-back on its way (settle: the framebuffer gets the view now, as
    /// Ui::gpu_settle does it; else it must hold it by itself)
    fn start_check(&mut self, ui: &mut Ui, what: &'static str, from: i32, settle: bool, ray: bool) {
        let i = ui.gpu_view_info(0, ID_VIEW).unwrap_or_default();
        if !i.device {
            let fb = ui.fb();
            let r = self.rect;
            let (x, y) = (r[0] + r[2] / 2, r[1] + r[3] / 2);
            let at = y as usize * fb.stride as usize + x as usize * 4;
            let p = fb.px[at] as u32 | (fb.px[at + 1] as u32) << 8 | (fb.px[at + 2] as u32) << 16;
            self.finish(p == PAINT & 0xFF_FFFF, &format!("{what}: no device ({}): the content paints ({p:06x})", i.error.as_deref().unwrap_or("")));
            return;
        }
        let c = self.content.borrow();
        let (Some(dev), Some(cam), size) = (c.dev.clone(), c.cam, c.size) else {
            drop(c);
            self.finish(false, &format!("{what}: the view has a device but no frame yet"));
            return;
        };
        drop(c);
        let r = self.rect;
        if (r[2] as u32, r[3] as u32) != size {
            self.finish(false, &format!("{what}: the view's rectangle {r:?} is not its frame's size {size:?}"));
            return;
        }
        let mut c = self.content.borrow_mut();
        let Content { r: rr, s, .. } = &mut *c;
        let (Some(rr), Some(s)) = (rr.as_mut(), s.as_mut()) else { return };
        let off = Off::new(&dev, size.0, size.1, GPU_VIEW_FORMAT).and_then(|mut off| off.submit(rr, s).map(|id| (off, id)));
        drop(c);
        match off {
            Ok((off, id)) => {
                self.check = Some(Check { what, off, rect: r, from, ray, cam, frame: i.frames, started: plat::time_us(), id, settle, ready_at: 0 });
                if settle {
                    ui.gpu_settle();
                }
                ui.timer(TIMER, 16);
            }
            Err(e) => self.finish(false, &format!("{what}: offscreen: {e}")),
        }
    }

    /// the check's next look: the offscreen image and the framebuffer with the view's newest frame -> done
    fn step_check(&mut self, ui: &mut Ui) -> bool {
        let Some(ck) = &self.check else { return true };
        let waited = plat::time_us().saturating_sub(ck.started);
        if let Some(d) = self.content.borrow().dev.as_ref() {
            let _ = d.poll();
            // (Vulkan: the frame must be done before its read-back is read; the browser's read says so itself)
            if !cfg!(target_arch = "wasm32") && !d.wait(ck.id, 5_000_000_000).unwrap_or(true) {
                return false;
            }
        }
        let want = match ck.off.read() {
            Ok(v) => v,
            Err(GpuError::NotReady) if waited < 5_000_000 => return false,
            Err(e) => {
                let what = ck.what;
                self.check = None;
                self.finish(false, &format!("{what}: the offscreen image: {e}"));
                return true;
            }
        };
        // (the browser: the view's newest frame comes to its CPU copy when its read-back has come, to the framebuffer with
        // the next settle: by hand, or at rest by itself, VIEW_REST_US after that render)
        let i = ui.gpu_view_info(0, ID_VIEW).unwrap_or_default();
        let now = plat::time_us();
        if i.read_frame < ck.frame && waited < 5_000_000 {
            return false;
        }
        // (the browser: cells with something over the view get the frame with the next render; a settle by hand after it)
        let (settle, ready_at) = (ck.settle, ck.ready_at);
        let render = if cfg!(target_arch = "wasm32") { 100_000 } else { 0 };
        if ready_at == 0 {
            if let Some(ck) = self.check.as_mut() {
                ck.ready_at = now;
            }
            if render > 0 || !settle {
                return false;
            }
        } else if now < ready_at + if settle { render } else { 400_000 } {
            return false;
        }
        if settle {
            ui.gpu_settle();
        }
        let Some(ck) = self.check.take() else { return true };
        let got = fb_rect(ui.fb(), ck.rect);
        let skip = (ck.from.max(0) * ck.rect[2] * 4) as usize;
        let (mut n, mut m) = (0u32, 0u8);
        for (p, q) in got[skip.min(got.len())..].chunks_exact(4).zip(want[skip.min(want.len())..].chunks_exact(4)) {
            if p[..3] != q[..3] {
                n += 1;
                m = m.max((0..3).map(|k| p[k].abs_diff(q[k])).max().unwrap_or(0));
            }
        }
        let rows = if ck.from > 0 { format!(", rows {} and below", ck.from) } else { String::new() };
        let mut fine = n == 0 && got.len() == want.len();
        let mut rays = String::new();
        if ck.ray {
            // the same scene through the rays, sRGB codes where render3d encodes
            let (geos, colors, specs) = world();
            let sn = sun();
            let mut rs = RefScene::new(sn.direction, sn.color.map(|v| v as f64), sn.ambient.map(|v| v as f64));
            for spec in &specs {
                rs.add(&geos[spec.mesh as usize], spec, colors[spec.material as usize]);
            }
            rs.srgb = RendererDesc::new(GPU_VIEW_FORMAT).encodes_srgb();
            let clear = CLEAR.map(|v| if rs.srgb { cpu::srgb_encode(v as f64) } else { (v * 255.0 + 0.5) as u8 });
            let c = ck.cam;
            let rc = RefCamera::new(c.eye, c.forward, c.up, c.fov_y, c.aspect, c.near, c.far);
            let rgba: Vec<u8> = got.chunks_exact(4).flat_map(|p| [p[2], p[1], p[0], 255]).collect();
            let cmp = cpu::compare(&rs, &rc, ck.rect[2] as u32, ck.rect[3] as u32, [clear[0], clear[1], clear[2]], &rgba, None);
            fine &= cmp.fine();
            rays = format!("; against the ray caster{}: {}", if rs.srgb { " (sRGB codes)" } else { "" }, cmp.text());
        }
        let o = ui.gpu_output(0);
        let error = i.error.as_deref().map(|e| format!(" (the view says: {e})")).unwrap_or_default();
        self.finish(fine, &format!("{}: {} x {}{rows}, {} ({:?}), generation {}, {} attach, apart {}, frame {} of {} in the framebuffer, {} read-backs{error}, {:.0} ms: \
                                    against offscreen {n} pixels differ (largest {m}){rays}", ck.what, ck.rect[2], ck.rect[3], o.adapter, i.level, i.generation,
                                   i.attaches, i.apart, i.read_frame, i.frames, i.reads, waited as f64 / 1000.0));
        true
    }

    fn finish(&mut self, ok: bool, what: &str) {
        self.checks += 1;
        self.failed += !ok as u32;
        say(&format!("check {}: {} {what}", self.checks, if ok { "ok" } else { "FAIL" }));
        self.result = format!("{} {}: {}", t("Prüfung"), self.checks, if ok { t("in Ordnung") } else { t("abweichend") });
    }

    /// (jfgui_GPU_VIEWCHECK) the next step when this one is done
    fn auto(&mut self, ui: &mut Ui) {
        let Some(k) = self.step else { return };
        let now = plat::time_us();
        let step = STEPS.get(k).copied().unwrap_or(Step::Done);
        let i = ui.gpu_view_info(0, ID_VIEW).unwrap_or_default();
        let o = ui.gpu_output(0);
        let next = match step {
            Step::Start => {
                // (the output decides first: with it the view draws on its device)
                let decided = !o.wanted || o.on || o.never;
                let ready = decided && i.device && i.frames >= 1 && (!o.on || i.apart);
                let without = decided && !i.device && now > self.since + 3_000_000;
                if ready || without || now > self.since + WAIT_US {
                    say(&format!("the view: device {}, output on {}, adapter {}, level {:?}, apart {}, frames {}, {:?}", i.device, o.on, o.adapter, i.level, i.apart,
                                 i.frames, i.error));
                    true
                } else {
                    false
                }
            }
            Step::Check { what, from, settle, ray } => {
                if self.left == 0 {
                    self.left = 1;
                    self.start_check(ui, what, from, settle, ray);
                }
                self.check.is_none()
            }
            Step::Hold(what) => {
                if self.left == 0 {
                    self.left = 1;
                    say(&format!("hold {what}"));
                }
                now > self.since + HOLD_US
            }
            Step::Turn(n) => {
                if self.left == 0 {
                    self.left = n + 1;
                }
                self.content.borrow_mut().angle += 0.04;
                self.ver = self.ver.wrapping_add(1);
                self.left -= 1;
                self.left == 1
            }
            Step::Rest(ms) => now > self.since + ms * 1000,
            Step::Over(on) => {
                self.over = on as i32;
                true
            }
            Step::Lose => {
                if !i.device {
                    // (no device, nothing to lose: the content paints, which the checks above saw)
                    say("no device: nothing to lose");
                    self.step = Some(k + 2);
                    self.since = now;
                    self.left = 0;
                    return;
                }
                self.generation = i.generation;
                say(&format!("lose the device (generation {})", i.generation));
                ui.gpu_test_lose();
                true
            }
            Step::Again => {
                // (a loss shows when the device is used: the view keeps turning until it is back, as an app would)
                let back = i.device && i.generation != self.generation && (!o.on || i.apart);
                if !back {
                    self.content.borrow_mut().angle += 0.04;
                    self.ver = self.ver.wrapping_add(1);
                }
                if back {
                    let c = self.content.borrow();
                    say(&format!("a new device: generation {} after {}, {} attaches, {} detaches of the content", i.generation, self.generation, c.attaches, c.detaches));
                    drop(c);
                    self.step = Some(k + 1);
                    self.since = now;
                    self.left = 0;
                    self.start_check(ui, "on the new device", 0, true, true);
                    return;
                }
                if now > self.since + WAIT_US {
                    self.finish(false, &format!("no new device in 20 s: generation {}, device {}, {:?}", i.generation, i.device, i.error));
                    true
                } else {
                    false
                }
            }
            Step::Done => {
                if self.check.is_none() {
                    say(&format!("done: {} checks, {} failed; {} presented images read back, {} differ", self.checks, self.failed, o.verified, o.verify_bad));
                    self.step = None;
                    self.left = u32::MAX; // (once)
                    // (a desktop program ends, Android's activity finishes; the browser's page stays as it is, for the test's look)
                    #[cfg(not(target_arch = "wasm32"))]
                    ui.quit();
                }
                return;
            }
        };
        if next {
            self.step = Some(k + 1);
            self.since = now;
            self.left = 0;
        }
    }

    /// (jfgui_GPU_VIEWBENCH) a frame of the measurement -> another one is wanted
    fn bench_step(&mut self, ui: &mut Ui) -> bool {
        let now = plat::time_us();
        let i = ui.gpu_view_info(0, ID_VIEW).unwrap_or_default();
        let o = ui.gpu_output(0);
        // the 3 s of warming up only before the first measured frame: bench_t0 is the measurement's start after it
        let warm = self.bench_base.is_some() || (now > self.bench_t0 + 3_000_000 && i.frames > 0 && (!o.wanted || o.on || o.never));
        let counters = [o.frames + o.cpu_frames, o.render_us, o.present_us, i.draw_us, i.frames];
        if warm {
            let Some(base) = self.bench_base else {
                self.bench_base = Some(counters);
                self.bench_last = (now, i.frames);
                self.bench_t0 = now;
                return self.turn();
            };
            if i.frames != self.bench_last.1 {
                self.bench_gaps.push(now - self.bench_last.0);
                self.bench_last = (now, i.frames);
            }
            if i.frames - base[4] >= self.bench as u64 {
                let f = (i.frames - base[4]).max(1) as f64;
                let per = |k: usize| (counters[k] - base[k]) as f64 / f;
                self.bench_gaps.sort_unstable();
                let med = self.bench_gaps.get(self.bench_gaps.len() / 2).copied().unwrap_or(0) as f64 / 1000.0;
                let mode = if !o.on { "the CPU path" } else if i.apart { "composed by the output" } else { "read back every frame" };
                say(&format!("bench {mode}, {} ({:?}), {} x {}: {} frames in {:.0} ms; per frame: interval {med:.3} ms (median), render {:.1} us, present {:.1} us, the view's draw {:.1} us, {} read-backs; {} frames presented; reserved {:.1} MiB", o.adapter, i.level, i.size.0, i.size.1, f as u64,
                             (now - self.bench_t0) as f64 / 1000.0, per(1), per(2), per(3), i.reads, counters[0] - base[0], o.reserved as f64 / 1048576.0));
                self.bench = 0;
                #[cfg(not(target_arch = "wasm32"))]
                ui.quit();
                return false;
            }
        }
        self.turn()
    }

    fn turn(&mut self) -> bool {
        self.content.borrow_mut().angle += 0.01;
        self.ver = self.ver.wrapping_add(1);
        true
    }

    /// the page's timer: a step of the animation, of the check, of the self-test
    pub fn command(&mut self, ui: &mut Ui, id: u32, arg: u32) -> bool {
        if id != UIC_TIMER || arg != TIMER {
            return false;
        }
        if self.bench > 0 {
            if self.bench_step(ui) {
                ui.timer(TIMER, 1);
            }
            ui.invalidate();
            return true;
        }
        if self.turning != 0 {
            self.content.borrow_mut().angle += 0.03;
            self.ver = self.ver.wrapping_add(1);
        }
        if self.check.is_some() {
            self.step_check(ui);
        }
        if self.step.is_some() {
            self.auto(ui);
        }
        if self.turning != 0 || self.check.is_some() || self.step.is_some() {
            ui.timer(TIMER, 16);
        }
        ui.invalidate();
        true
    }
}
