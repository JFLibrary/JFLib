// view -- views of GPU content in real windows (lib/ui/gpu_view.rs, G5e1; docs/GPU.md "3D-Ansicht"): a render3d
// scene between the test's bands, under popups and tooltips, beside a second view and in a second window; its
// pixels in the framebuffer against render3d offscreen on the same device and against render3dtest's ray caster
// (cpu.rs and common.rs are render3dtest's, included as they are).  The connection to render3d lives here, in the
// app (View3d): lib/ui knows nothing of render3d.
//
//   view       the device (the output's, Voll with AF_GPU_3D), the first frame drawn by the output's pass, the
//              framebuffer = render3d offscreen = the ray caster, the screen; idle: no frame; an animation; something
//              of the base over the view; a popup and a tooltip over it while it moves
//   views2     two views in a window, a view in a second window on the same device; the second window closes
//   viewsize   a new size (the CPU path meanwhile), 150 % and back, minimised (no frame while nobody sees it)
//   viewlose   Device::test_lose while the view moves: the CPU path with the last frame, the content detached, a new
//              device: attached again, its renderer built anew, the same picture
//   viewplain  without reading every presented image back: during an animation the framebuffer lacks the view,
//              Ui::gpu_settle gives it, at rest it holds it by itself; a popup over it reads every frame back
//   viewcpu    jfgui_GPU_OUTPUT=0: the windows on the CPU path, the views on a device of their own (Voll), every
//              frame read back; the window's DC shows it
//   viewoff    jfgui_GPU=0: no device, the content paints
//   viewbasis  without AF_GPU_3D: the UI's Basis device, the same pictures
//
//   uigputest --bench view [--n N] [--basis] [--idle] [--timer MS] [--offscreen [--read]]   a window of 1280 x 800, a view of
//              1200 x 700 of render3dtest's gallery (3000 instances), turning: per frame the CPU of render and present,
//              of the view's frame and its read-backs, the GPU of render3d's frame (timestamps), memory.  Modes by
//              the environment as ever: jfgui_GPU_VIEWS=0 (every frame read back), jfgui_GPU_OUTPUT=0 (the CPU
//              path); --offscreen: render3d alone, no window (--read: and the read-back the CPU path pays)

use alloc::format;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::cell::RefCell;

use gpu::{ColorAttachment, CommandEncoder, Device, Format, GpuError, Level, Load, SubmitId};
use render3d::{Camera, Material, Renderer, RendererDesc, Scene as Scene3, Sun};
use ui::*;

use crate::common::{ball, cube, gallery_fill, gallery_specs, plate, quat, Geo, Off, Spec, CLEAR, CLEAR8};
use crate::cpu::{self, RefCamera, RefScene};
use crate::{out, pixel_becomes, Test};

pub const VIEW_ID: u32 = 800;
pub const VIEW_ID2: u32 = 801;
/// the view in the second window
pub const VIEW_ID3: u32 = 802;
/// what View3d paints without a device
pub const PAINT: u32 = 0xFF55_2266;

/// the views a test draws (Scene::views)
#[derive(Default)]
pub struct ViewSc {
    pub one: Option<Rc<RefCell<View3d>>>,
    pub ver: u32,
    /// something of the base over the view's corner
    pub hud: bool,
    pub two: Option<Rc<RefCell<View3d>>>,
    pub ver2: u32,
    /// a view in the second window
    pub win2: Option<Rc<RefCell<View3d>>>,
    pub ver3: u32,
}

/// the first view's rectangle in a w x h window (it follows the window's size)
pub fn view_rect(w: i32, h: i32) -> [i32; 4] {
    [40, 50, (w - 80).max(16), (h - 140).max(16)]
}

/// the second view, below the first
pub fn view2_rect(_w: i32, h: i32) -> [i32; 4] {
    [44, h - 82, 232, 70]
}

/// the view in the second window
pub const VIEW3_RECT: [i32; 4] = [20, 20, 300, 180];

/// the scene of a view: a few shapes on a floor (ray-castable), or render3dtest's gallery (3000 instances)
pub enum Kind {
    Small,
    Gallery,
}

/// what the ray caster needs to know of the small scene
pub struct World {
    pub geos: Vec<Geo>,
    pub specs: Vec<Spec>,
    pub colors: Vec<[f32; 3]>,
}

pub fn world() -> World {
    let floor = quat([1.0, 0.0, 0.0], -core::f64::consts::FRAC_PI_2);
    let none = [0.0f32, 0.0, 0.0, 1.0];
    let spin = quat(cpu::norm([1.0, 2.0, 0.5]), 0.9);
    let at = |mesh: u32, material: u32, pos: [f64; 3], rot: [f32; 4], scale: [f32; 3]| Spec { mesh, material, pos, rot, scale };
    World {
        geos: vec![cube(), ball(12, 18), plate()],
        colors: vec![[1.0, 0.5, 0.25], [0.25, 1.0, 0.5], [0.5, 0.25, 1.0], [0.85, 0.85, 0.8]],
        specs: vec![
            at(2, 3, [0.0, -1.0, 0.0], floor, [6.0, 6.0, 1.0]),
            at(0, 0, [-2.2, 0.0, 0.0], spin, [1.0; 3]),
            at(1, 1, [2.2, 0.0, 0.5], none, [1.2; 3]),
            at(0, 2, [0.0, -0.4, -2.5], none, [0.6; 3]),
            at(1, 0, [0.4, 0.1, 2.2], none, [0.5, 1.1, 0.5]),
        ],
    }
}

pub fn sun() -> Sun {
    Sun { direction: [0.3, 0.8, 0.5], color: [0.7 * core::f32::consts::PI; 3], ambient: [0.3; 3] }
}

/// an orbit around the scene: angle a (radians), for a target of w x h
pub fn orbit(kind: &Kind, a: f64, w: u32, h: u32) -> Camera {
    let (s, c) = cpu::sincos(a);
    let aspect = w as f64 / h.max(1) as f64;
    match kind {
        Kind::Small => Camera::look_at([7.5 * s, 4.0, 7.5 * c], [0.0, 0.0, 0.0], [0.0, 1.0, 0.0], 0.9, aspect, 0.1, f64::INFINITY),
        Kind::Gallery => Camera::look_at([90.0 * s, 12.0, 90.0 * c - 15.0], [0.0, 0.0, -15.0], [0.0, 1.0, 0.0], 1.0, aspect, 0.1, f64::INFINITY),
    }
}

/// render3d in a view: a Renderer and a Scene on the device lib/ui hands over (the whole connection)
pub struct View3d {
    pub kind: Kind,
    pub angle: f64,
    pub stats: bool,
    pub dev: Option<Device>,
    pub r: Option<Renderer>,
    pub s: Option<Scene3>,
    /// the camera and size of the last frame
    pub cam: Option<Camera>,
    pub size: (u32, u32),
    pub attaches: u32,
    pub detaches: u32,
    pub draws: u32,
    /// render3d's GPU time per frame (ms), when stats is on
    pub gpu_ms: Vec<f64>,
}

impl View3d {
    pub fn new(kind: Kind, angle: f64) -> Rc<RefCell<View3d>> {
        Rc::new(RefCell::new(View3d { kind, angle, stats: false, dev: None, r: None, s: None, cam: None, size: (0, 0), attaches: 0, detaches: 0, draws: 0, gpu_ms: Vec::new() }))
    }
}

impl GpuContent for View3d {
    fn attach(&mut self, dev: &Device) -> gpu::Result<()> {
        let mut r = Renderer::new(dev, &RendererDesc::new(GPU_VIEW_FORMAT))?;
        let mut s;
        match self.kind {
            Kind::Small => {
                let w = world();
                for g in &w.geos {
                    r.add_mesh(&g.data())?;
                }
                for c in &w.colors {
                    r.add_material(&Material::color(c[0], c[1], c[2]))?;
                }
                s = Scene3::new(&r)?;
                for spec in &w.specs {
                    s.add(spec.instance())?;
                }
            }
            Kind::Gallery => {
                let (meshes, materials) = gallery_fill(&mut r)?;
                s = Scene3::new(&r)?;
                for spec in gallery_specs(meshes, materials, crate::common::GALLERY_N, [0.0; 3]) {
                    s.add(spec.instance())?;
                }
            }
        }
        s.set_sun(&sun());
        s.set_stats(self.stats);
        self.r = Some(r);
        self.s = Some(s);
        self.dev = Some(dev.clone());
        self.attaches += 1;
        Ok(())
    }

    fn detach(&mut self) {
        self.s = None;
        self.r = None;
        self.dev = None;
        self.detaches += 1;
    }

    fn draw<'a>(&'a mut self, e: &mut CommandEncoder<'a>, target: ColorAttachment<'a>, size: (u32, u32)) -> gpu::Result<()> {
        let cam = orbit(&self.kind, self.angle, size.0, size.1);
        {
            let (Some(r), Some(s)) = (self.r.as_mut(), self.s.as_mut()) else { return Err(GpuError::Invalid("View3d: not attached")) };
            r.set_size(size.0, size.1)?;
            s.set_camera(&cam);
            s.prepare(r)?;
            if self.stats {
                for f in s.take_stats() {
                    if let Some(st) = f.stamps {
                        self.gpu_ms.push(st[3] - st[0]);
                    }
                }
            }
        }
        self.cam = Some(cam);
        self.size = size;
        self.draws += 1;
        let me: &'a View3d = self;
        let (Some(r), Some(s)) = (me.r.as_ref(), me.s.as_ref()) else { return Err(GpuError::Invalid("View3d: not attached")) };
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

/// the rectangle r of the framebuffer, rows of B, G, R, x
fn fb_rect(fb: &gfx::Surface, r: [i32; 4]) -> Vec<u8> {
    let mut v = Vec::with_capacity((r[2] * r[3] * 4) as usize);
    for y in r[1]..r[1] + r[3] {
        let at = y as usize * fb.stride as usize + r[0] as usize * 4;
        v.extend_from_slice(&fb.px[at..at + r[2] as usize * 4]);
    }
    v
}

/// pixels whose colour differs (the fourth byte means nothing), the largest difference of a channel
fn rgb_diff(a: &[u8], b: &[u8]) -> (u32, u8) {
    if a.len() != b.len() {
        return (u32::MAX, 255);
    }
    let (mut n, mut m) = (0u32, 0u8);
    for (p, q) in a.chunks_exact(4).zip(b.chunks_exact(4)) {
        if p[..3] != q[..3] {
            n += 1;
            for k in 0..3 {
                m = m.max(p[k].abs_diff(q[k]));
            }
        }
    }
    (n, m)
}

/// the content's scene offscreen, in the view's format and size, with the same renderer on the same device: what
/// render3d draws without lib/ui (B, G, R, A rows)
fn offscreen(content: &Rc<RefCell<View3d>>) -> Option<Vec<u8>> {
    let mut c = content.borrow_mut();
    let (w, h) = c.size;
    let View3d { dev, r, s, .. } = &mut *c;
    let (dev, r, s) = (dev.clone()?, r.as_mut()?, s.as_mut()?);
    let mut off = Off::new(&dev, w, h, Format::Bgra8Unorm).ok()?;
    let id = off.submit(r, s).ok()?;
    if !dev.wait(id, 10_000_000_000).ok()? {
        return None;
    }
    off.read().ok()
}

impl Test {
    /// the view's pixels in the framebuffer (after Ui::gpu_settle when settle) against render3d offscreen and, for the
    /// small scene, the ray caster -> they are the same
    pub fn view_check(&mut self, ui: &mut Ui, content: &Rc<RefCell<View3d>>, r: [i32; 4], settle: bool, ray: bool, what: &str) -> bool {
        self.view_check_rows(ui, content, r, 0, settle, ray, what)
    }

    /// the same from row `from` of the view on (what lies over its top rows is left out)
    #[allow(clippy::too_many_arguments)]
    pub fn view_check_rows(&mut self, ui: &mut Ui, content: &Rc<RefCell<View3d>>, r: [i32; 4], from: i32, settle: bool, ray: bool, what: &str) -> bool {
        if settle {
            ui.gpu_settle();
        }
        let got = fb_rect(ui.fb(), r);
        let Some(want) = offscreen(content) else {
            self.ok(false, &format!("{what}: no offscreen image (the view has no device?)"));
            return false;
        };
        let skip = (from.max(0) * r[2] * 4) as usize;
        let (n, m) = rgb_diff(&got[skip.min(got.len())..], &want[skip.min(want.len())..]);
        let size = content.borrow().size;
        let rows = if from > 0 { format!(", rows {from} and below") } else { String::new() };
        let same = self.ok(n == 0, &format!("{what}: the view in the framebuffer ({} x {}{rows}) against render3d offscreen: {n} pixels differ (largest {m})", size.0, size.1));
        if ray && matches!(content.borrow().kind, Kind::Small) {
            let w = world();
            let sn = sun();
            let mut rs = RefScene::new(sn.direction, sn.color.map(|v| v as f64), sn.ambient.map(|v| v as f64));
            for spec in &w.specs {
                rs.add(&w.geos[spec.mesh as usize], spec, w.colors[spec.material as usize]);
            }
            // (G5e2) the view's 8-bit target: render3d encodes sRGB, the clear colour too; the rays give sRGB codes
            rs.srgb = content.borrow().r.as_ref().is_some_and(|r| r.desc().encodes_srgb());
            let clear = if rs.srgb { CLEAR.map(|v| cpu::srgb_encode(v as f64)) } else { [CLEAR8[0], CLEAR8[1], CLEAR8[2], 255] };
            let Some(c) = content.borrow().cam else { return same };
            let rc = RefCamera::new(c.eye, c.forward, c.up, c.fov_y, c.aspect, c.near, c.far);
            let rgba: Vec<u8> = got.chunks_exact(4).flat_map(|p| [p[2], p[1], p[0], 255]).collect();
            let cmp = cpu::compare(&rs, &rc, size.0, size.1, [clear[0], clear[1], clear[2]], &rgba, None);
            self.ok(cmp.fine(), &format!("{what}: against the ray caster{}: {}", if rs.srgb { " (sRGB codes)" } else { "" }, cmp.text()));
        }
        same
    }

    /// the composed screen at the view's middle = the framebuffer there (another window on top: a note)
    fn view_screen(&mut self, ui: &mut Ui, r: [i32; 4], what: &str) {
        ui.gpu_settle();
        let (x, y) = (r[0] + r[2] / 2, r[1] + r[3] / 2);
        let fb = ui.fb();
        let at = y as usize * fb.stride as usize + x as usize * 4;
        let want = fb.px[at] as u32 | (fb.px[at + 1] as u32) << 8 | (fb.px[at + 2] as u32) << 16;
        let mut got = crate::shown_px(x, y);
        let t0 = plat::time_us();
        while got.is_some_and(|g| g != want) && plat::time_us() < t0 + 1_500_000 {
            plat::sleep_ms(4);
            plat::gpu_wait(plat::time_us());
            got = crate::shown_px(x, y);
        }
        match got {
            None => out(&format!("note: {what}: no way to read the screen here (no result)")),
            Some(g) if g != want && plat::win_test(plat::WT_FROM_POINT, 0, [x, y, 0, 0]) != 1 => {
                out(&format!("note: {what}: another window lies on top of ours (no result)"));
            }
            Some(g) => {
                self.ok(g == want, &format!("{what}: the screen shows {g:06x} in the view's middle, the framebuffer {want:06x}"));
            }
        }
    }

    fn view_info(&self, ui: &Ui, win: u32, id: u32) -> GpuViewInfo {
        ui.gpu_view_info(win, id).unwrap_or_default()
    }

    /// a step of an animation: the view turns, n passes -> done
    fn view_turn(&mut self, n: u32) -> bool {
        if let Some(c) = &self.sc.views.one {
            c.borrow_mut().angle += 0.04;
            self.sc.views.ver += 1;
        }
        if let Some(c) = &self.sc.views.two {
            c.borrow_mut().angle -= 0.05;
            self.sc.views.ver2 += 1;
        }
        if let Some(c) = &self.sc.views.win2 {
            c.borrow_mut().angle += 0.03;
            self.sc.views.ver3 += 1;
        }
        self.mode += 1;
        if self.mode >= n {
            self.mode = 0;
            return true;
        }
        false
    }

    fn one(&self) -> Rc<RefCell<View3d>> {
        self.sc.views.one.clone().unwrap_or_else(|| View3d::new(Kind::Small, 0.0))
    }

    pub fn t_view(&mut self, ui: &mut Ui) {
        let r = view_rect(ui.fb().w, ui.fb().h);
        let o = ui.gpu_output(0);
        match self.pc {
            0 => {
                if self.wait_on(ui, 15_000) == Some(true) {
                    self.sc.views.one = Some(View3d::new(Kind::Small, 0.3));
                    self.sc.views.ver = 1;
                    self.next();
                }
            }
            1 => {
                if self.after(150) {
                    let i = self.view_info(ui, 0, VIEW_ID);
                    out(&format!("  {} ({:?}), view {:?}", o.adapter, i.level, i.size));
                    self.ok(i.device && i.output && i.attaches == 1, &format!("the view on the output's device ({} attach)", i.attaches));
                    self.ok(i.level == Some(Level::Voll), &format!("AF_GPU_3D: the device at level Voll ({:?})", i.level));
                    self.ok(i.apart && o.view_quads == 1 && i.frames >= 1, &format!("drawn by the output's render pass ({} quad, {} frames)", o.view_quads, i.frames));
                    let c = self.one();
                    self.view_check(ui, &c, r, true, true, "the first frame");
                    self.view_screen(ui, r, "the first frame");
                    self.same_so_far(ui, "the first frame");
                    let i = self.view_info(ui, 0, VIEW_ID);
                    self.mark = [o.frames, i.frames, i.reads, plat::win_test_usage().0];
                    self.next();
                }
            }
            2 => {
                // idle: no frame of the window, none of the view
                if self.after(600) {
                    let i = self.view_info(ui, 0, VIEW_ID);
                    let cpu = plat::win_test_usage().0 - self.mark[3];
                    self.ok(o.frames == self.mark[0] && i.frames == self.mark[1], &format!("idle 600 ms: {} frames presented, {} of the view", o.frames - self.mark[0], i.frames - self.mark[1]));
                    self.ok(i.reads <= self.mark[2] + 1, &format!("idle: {} read-backs", i.reads - self.mark[2]));
                    out(&format!("  CPU time over 600 ms: {:.2} ms (the test's own timer included)", cpu as f64 / 1000.0));
                    self.mark = [o.frames, i.frames, i.reads, 0];
                    self.next();
                }
            }
            3 => {
                // it turns: a frame of the view per pass, the window's frames follow
                if self.view_turn(40) {
                    let i = self.view_info(ui, 0, VIEW_ID);
                    // (a frame per render, a render for one or two passes)
                    self.ok(i.frames >= self.mark[1] + 10 && o.frames > self.mark[0], &format!("an animation: {} frames of the view, {} presented", i.frames - self.mark[1], o.frames - self.mark[0]));
                    self.ok(i.apart && o.view_quads == 1, "drawn by the output's render pass while it moves");
                    self.same_so_far(ui, "an animation");
                    self.next();
                }
            }
            4 => {
                if self.after(400) {
                    let c = self.one();
                    self.view_check(ui, &c, r, false, true, "at rest after the animation (not settled by hand)");
                    self.view_screen(ui, r, "at rest");
                    self.sc.views.hud = true;
                    let i = self.view_info(ui, 0, VIEW_ID);
                    self.mark[2] = i.reads;
                    self.next();
                }
            }
            5 => {
                // something of the base over it: those cells drawn by the CPU over the frame read back
                if self.view_turn(20) {
                    let i = self.view_info(ui, 0, VIEW_ID);
                    self.ok(i.apart && o.view_quads == 1, "something over the view: still drawn by the output");
                    out(&format!("  {} read-backs for 20 frames with something over the view", i.reads - self.mark[2]));
                    self.same_so_far(ui, "something of the base over the view while it moves");
                    self.next();
                }
            }
            6 => {
                if self.after(100) {
                    let c = self.one();
                    self.view_check_rows(ui, &c, r, 60, true, false, "below what lies over it");
                    self.sc.mask = 2; // a popup over the view
                    self.next();
                }
            }
            7 => {
                if self.view_turn(20) {
                    self.ok(o.layer_textures == 1 && o.view_quads == 1, &format!("a popup over the view while it moves: {} layer, {} view", o.layer_textures, o.view_quads));
                    self.same_so_far(ui, "a popup over the view while it moves");
                    self.sc.mask = 4; // a tooltip
                    self.next();
                }
            }
            8 => {
                if self.after(100) {
                    self.same_so_far(ui, "a tooltip over the view");
                    self.sc.mask = 0;
                    self.sc.views.hud = false;
                    self.next();
                }
            }
            9 => {
                if self.after(300) {
                    let c = self.one();
                    self.view_check(ui, &c, r, false, true, "all closed, at rest");
                    self.view_screen(ui, r, "all closed");
                    self.same_so_far(ui, "all closed");
                    let n = c.borrow().attaches;
                    self.ok(n == 1, &format!("one renderer for all of it ({n} attach)"));
                    self.finish(ui);
                }
            }
            _ => {}
        }
    }

    pub fn t_views2(&mut self, ui: &mut Ui) {
        let (fw, fh) = (ui.fb().w, ui.fb().h);
        let o = ui.gpu_output(0);
        match self.pc {
            0 => {
                if self.wait_on(ui, 15_000) == Some(true) {
                    self.sc.views.one = Some(View3d::new(Kind::Small, 0.3));
                    self.sc.views.two = Some(View3d::new(Kind::Small, 2.1));
                    (self.sc.views.ver, self.sc.views.ver2) = (1, 1);
                    self.next();
                }
            }
            1 => {
                if self.after(150) {
                    let (a, b) = (self.view_info(ui, 0, VIEW_ID), self.view_info(ui, 0, VIEW_ID2));
                    self.ok(o.view_quads == 2 && a.apart && b.apart, &format!("two views drawn by the output ({} quads)", o.view_quads));
                    self.ok(a.generation == b.generation && a.output && b.output, "both on the output's device");
                    let (c1, c2) = (self.one(), self.sc.views.two.clone().unwrap_or_else(|| View3d::new(Kind::Small, 0.0)));
                    self.view_check(ui, &c1, view_rect(fw, fh), true, true, "the first view");
                    self.view_check(ui, &c2, view2_rect(fw, fh), true, true, "the second view");
                    self.same_so_far(ui, "two views");
                    // a view in a second window
                    self.sc.views.win2 = Some(View3d::new(Kind::Small, 4.0));
                    self.sc.views.ver3 = 1;
                    self.second.set(1);
                    self.next();
                }
            }
            2 => {
                let w2 = ui.gpu_output(1);
                let c = self.view_info(ui, 1, VIEW_ID3);
                if w2.on && c.frames >= 1 && c.apart {
                    let a = self.view_info(ui, 0, VIEW_ID);
                    self.ok(c.output && c.generation == a.generation, &format!("a view in the second window on the same device (generation {} and {})", c.generation, a.generation));
                    self.ok(w2.view_quads == 1, "drawn by the second window's render pass");
                    self.next();
                } else if self.after(8000) {
                    self.ok(false, &format!("the second window's view: on {}, {} frames, apart {}, {:?}", w2.on, c.frames, c.apart, c.error));
                    self.finish(ui);
                }
            }
            3 => {
                if self.view_turn(30) {
                    let w2 = ui.gpu_output(1);
                    self.same_so_far(ui, "three views turning, the first window");
                    self.ok(w2.verified > 0 && w2.verify_bad == 0, &format!("the second window: {} images read back, {} differ", w2.verified, w2.verify_bad));
                    self.next();
                }
            }
            4 => {
                if self.after(300) {
                    let (c1, c2) = (self.one(), self.sc.views.two.clone().unwrap_or_else(|| View3d::new(Kind::Small, 0.0)));
                    self.view_check(ui, &c1, view_rect(fw, fh), false, true, "the first view at rest");
                    self.view_check(ui, &c2, view2_rect(fw, fh), false, false, "the second view at rest");
                    self.second.set(0);
                    self.next();
                }
            }
            5 => {
                if self.after(300) {
                    self.mark[0] = self.view_info(ui, 1, VIEW_ID3).frames;
                    self.next();
                }
            }
            6 => {
                // the second window hidden: its view keeps its device, draws no frame
                if self.view_turn(10) {
                    let c = self.view_info(ui, 1, VIEW_ID3);
                    let d = self.sc.views.win2.as_ref().map_or(0, |c| c.borrow().detaches);
                    self.ok(c.frames == self.mark[0] && c.device && d == 0, &format!("the second window hidden: {} frames of its view, still attached ({})", c.frames - self.mark[0], c.device));
                    self.ok(ui.gpu_output(0).on && self.view_info(ui, 0, VIEW_ID).apart, "the first window goes on");
                    self.same_so_far(ui, "after the second window");
                    self.finish(ui);
                }
            }
            _ => {}
        }
    }

    pub fn t_viewsize(&mut self, ui: &mut Ui) {
        let r = view_rect(ui.fb().w, ui.fb().h);
        let o = ui.gpu_output(0);
        match self.pc {
            0 => {
                if self.wait_on(ui, 15_000) == Some(true) {
                    self.sc.views.one = Some(View3d::new(Kind::Small, 0.3));
                    self.sc.views.ver = 1;
                    self.next();
                }
            }
            1 => {
                if self.after(150) {
                    let c = self.one();
                    self.view_check(ui, &c, r, true, true, "before");
                    let (x, y) = plat::win_get_rect(0).map_or((100, 100), |r| (r[0], r[1]));
                    plat::win_test(plat::WT_MOVE, 0, [x, y, 760, 540]);
                    self.mark[0] = self.view_info(ui, 0, VIEW_ID).reads;
                    self.next();
                }
            }
            2 => {
                if ui.fb().w == 760 && o.leaves >= 1 && !o.on {
                    // (on the CPU path: every frame of the view read back at once)
                    let i = self.view_info(ui, 0, VIEW_ID);
                    let c = self.one();
                    self.ok(!i.apart && i.size == (r[2] as u32, r[3] as u32), &format!("a new size: the CPU path, the view {:?}", i.size));
                    self.view_check(ui, &c, r, false, false, "a new size, on the CPU path");
                    self.next();
                } else if self.after(3000) {
                    self.ok(false, "a new size: no CPU frame");
                    self.finish(ui);
                }
            }
            3 => {
                if self.wait_on(ui, 5000) == Some(true) {
                    self.next();
                }
            }
            4 => {
                if self.after(150) {
                    let i = self.view_info(ui, 0, VIEW_ID);
                    let c = self.one();
                    self.ok(i.apart, "the swapchain again: the output draws the view");
                    self.view_check(ui, &c, r, true, true, "at the new size");
                    self.same_so_far(ui, "a new size");
                    self.mark[1] = ui.fb().w as u64;
                    if plat::win_test(plat::WT_DPI, 0, [144, 0, 0, 0]) < 0 {
                        out("note: no second scale for a window here: 150 % left out");
                        self.pc = 9;
                    } else {
                        self.next();
                    }
                }
            }
            5 | 7 => {
                // (the new frame first, then the swapchain again)
                let at = ui.fb().w as u64;
                let reached = if self.pc == 5 { at > self.mark[1] } else { at == self.mark[1] };
                if reached {
                    if self.wait_on(ui, 8000) == Some(true) {
                        // (G5e2) a frame of the view after the swapchain's first one, which is whole and so never
                        // apart: on Wayland nothing else may follow it (the Fedora's KWin, 150 %)
                        let _ = self.view_turn(u32::MAX);
                        self.mode = 0;
                        self.next();
                    }
                } else if self.after(8000) {
                    self.ok(false, &format!("{}: the frame is {} wide", if self.pc == 5 { "150 %" } else { "100 %" }, ui.fb().w));
                    self.finish(ui);
                }
            }
            6 => {
                if self.after(150) {
                    let i = self.view_info(ui, 0, VIEW_ID);
                    // (G5e2: KWin under Wayland at 150 %: the first swapchain of the new scale goes stale at once, the
                    // window is on the CPU path for a frame and switches again; the steady state is what counts)
                    if !(i.apart && o.on) && !self.after(5000) {
                        let _ = self.view_turn(u32::MAX);
                        self.mode = 0;
                        return;
                    }
                    let c = self.one();
                    self.ok(i.apart && i.size == (r[2] as u32, r[3] as u32), &format!("150 %: the view {:?}, drawn by the output: {}", i.size, i.apart));
                    self.view_check(ui, &c, r, true, false, "150 %");
                    plat::win_test(plat::WT_DPI, 0, [96, 0, 0, 0]);
                    self.next();
                }
            }
            8 => {
                if self.after(150) {
                    let c = self.one();
                    self.view_check(ui, &c, r, true, false, "100 % again");
                    self.same_so_far(ui, "150 % and back");
                    self.next();
                }
            }
            9 => {
                plat::win_test(plat::WT_SHOW, 0, [crate::SW_SHOWMINNOACTIVE, 0, 0, 0]);
                self.next();
            }
            10 => {
                if plat::win_state(0) & plat::WST_MINIMIZED != 0 {
                    self.mark[2] = self.view_info(ui, 0, VIEW_ID).frames;
                    self.mode = 0;
                    self.next();
                } else if self.after(3000) {
                    self.ok(false, "the window did not minimise");
                    self.finish(ui);
                }
            }
            11 => {
                // it would turn, nobody sees it: no frame
                if self.view_turn(10) {
                    let i = self.view_info(ui, 0, VIEW_ID);
                    self.ok(i.frames == self.mark[2], &format!("minimised: {} frames of the view", i.frames - self.mark[2]));
                    plat::win_test(plat::WT_SHOW, 0, [crate::SW_SHOWNOACTIVATE, 0, 0, 0]);
                    self.next();
                }
            }
            12 => {
                if plat::win_state(0) & plat::WST_MINIMIZED == 0 && self.wait_on(ui, 5000) == Some(true) {
                    self.next();
                } else if self.after(8000) {
                    self.ok(false, "not back on the swapchain after the restore");
                    self.finish(ui);
                }
            }
            13 => {
                if self.after(200) {
                    let i = self.view_info(ui, 0, VIEW_ID);
                    let c = self.one();
                    self.ok(i.frames > self.mark[2] && i.apart, &format!("restored: the frame it missed ({} new), drawn by the output: {}", i.frames - self.mark[2], i.apart));
                    self.view_check(ui, &c, r, true, true, "restored");
                    self.view_screen(ui, r, "restored");
                    self.finish(ui);
                }
            }
            _ => {}
        }
    }

    pub fn t_viewlose(&mut self, ui: &mut Ui) {
        let r = view_rect(ui.fb().w, ui.fb().h);
        let o = ui.gpu_output(0);
        match self.pc {
            0 => {
                if self.wait_on(ui, 15_000) == Some(true) {
                    self.sc.views.one = Some(View3d::new(Kind::Small, 0.3));
                    self.sc.views.ver = 1;
                    self.next();
                }
            }
            1 => {
                if self.after(150) {
                    let c = self.one();
                    self.view_check(ui, &c, r, true, true, "before");
                    self.mark[0] = self.view_info(ui, 0, VIEW_ID).generation as u64;
                    ui.gpu_test_lose();
                    self.next();
                }
            }
            2 => {
                // the view's next frame finds the device lost
                if !o.on && o.leaves >= 1 {
                    let i = self.view_info(ui, 0, VIEW_ID);
                    let c = self.one();
                    let d = c.borrow().detaches;
                    self.ok(!i.device && d == 1, &format!("lost: the content detached ({d}), no device ({})", i.device));
                    self.ok(i.detaches == 1, &format!("the view says so: {} detach, {:?}", i.detaches, i.error));
                    self.next();
                } else {
                    let _ = self.view_turn(u32::MAX);
                    if self.after(5000) {
                        self.ok(false, "the device loss was not noticed");
                        self.finish(ui);
                    }
                }
            }
            3 => {
                // until a new device: the last frame read back stays in the framebuffer (not the painting)
                if self.after(100) {
                    let fb = ui.fb();
                    let (x, y) = (r[0] + r[2] / 2, r[1] + r[3] / 2);
                    let p = u32::from_le_bytes(fb.px[y as usize * fb.stride as usize + x as usize * 4..][..4].try_into().unwrap_or([0; 4])) & 0xFF_FFFF;
                    self.ok(p != PAINT & 0xFF_FFFF, &format!("without a device the view keeps its last frame ({p:06x}), not the painting"));
                    self.next();
                }
            }
            4 => {
                if self.wait_on(ui, 15_000) == Some(true) {
                    self.next();
                }
            }
            5 => {
                if self.after(200) {
                    let i = self.view_info(ui, 0, VIEW_ID);
                    let c = self.one();
                    let a = c.borrow().attaches;
                    self.ok(i.device && i.output && i.generation as u64 > self.mark[0], &format!("a new device: generation {} after {}", i.generation, self.mark[0]));
                    self.ok(a == 2 && i.attaches == 2, &format!("attached again: the renderer built anew ({a} attaches)"));
                    self.ok(i.apart, "drawn by the output again");
                    self.view_check(ui, &c, r, true, true, "on the new device");
                    self.view_screen(ui, r, "on the new device");
                    self.next();
                }
            }
            6 => {
                if self.view_turn(20) {
                    self.same_so_far(ui, "it turns on the new device");
                    self.finish(ui);
                }
            }
            _ => {}
        }
    }

    pub fn t_viewplain(&mut self, ui: &mut Ui) {
        let r = view_rect(ui.fb().w, ui.fb().h);
        match self.pc {
            0 => {
                if self.wait_on(ui, 15_000) == Some(true) {
                    self.sc.views.one = Some(View3d::new(Kind::Small, 0.3));
                    self.sc.views.ver = 1;
                    self.next();
                }
            }
            1 => {
                // the first frame: at rest its pixels come into the framebuffer by themselves
                if self.after(500) {
                    let c = self.one();
                    let i = self.view_info(ui, 0, VIEW_ID);
                    self.ok(i.apart && i.reads >= 1, &format!("at rest: {} read-back(s) by themselves", i.reads));
                    self.view_check(ui, &c, r, false, true, "at rest, nobody asked");
                    self.mark[0] = i.reads;
                    self.next();
                }
            }
            2 => {
                if self.view_turn(30) {
                    // (no read-back while it turns: the framebuffer lacks the view)
                    let i = self.view_info(ui, 0, VIEW_ID);
                    self.ok(i.reads == self.mark[0], &format!("30 frames turning: {} read-backs", i.reads - self.mark[0]));
                    let c = self.one();
                    let got = fb_rect(ui.fb(), r);
                    if let Some(want) = offscreen(&c) {
                        let (n, _) = rgb_diff(&got, &want);
                        self.ok(n > 0, &format!("while it turns the framebuffer holds an older frame ({n} pixels differ)"));
                    }
                    self.view_check(ui, &c, r, true, false, "Ui::gpu_settle while it turns");
                    let i = self.view_info(ui, 0, VIEW_ID);
                    self.ok(i.reads == self.mark[0] + 1, &format!("Ui::gpu_settle: {} read-back", i.reads - self.mark[0]));
                    self.mark[0] = i.reads;
                    self.next();
                }
            }
            3 => {
                if self.view_turn(10) {
                    self.next();
                }
            }
            4 => {
                if self.after(500) {
                    let c = self.one();
                    let i = self.view_info(ui, 0, VIEW_ID);
                    self.ok(i.reads == self.mark[0] + 1, &format!("at rest again: {} read-back", i.reads - self.mark[0]));
                    self.view_check(ui, &c, r, false, true, "at rest again, nobody asked");
                    self.sc.mask = 2;
                    self.mark[0] = i.reads;
                    self.mark[1] = i.frames;
                    self.next();
                }
            }
            5 => {
                // a popup over it: its layer holds what lies below, every frame read back
                if self.view_turn(10) {
                    let i = self.view_info(ui, 0, VIEW_ID);
                    let (n, f) = (i.reads - self.mark[0], i.frames - self.mark[1]);
                    self.ok(f >= 3 && n + 1 >= f, &format!("a popup over it while it turns: {n} read-backs for {f} frames"));
                    self.next();
                }
            }
            6 => {
                if self.after(500) {
                    let c = self.one();
                    self.view_check_rows(ui, &c, r, 260, false, false, "below the popup at rest");
                    self.sc.mask = 0;
                    self.next();
                }
            }
            7 => {
                if self.after(300) {
                    let c = self.one();
                    self.view_check(ui, &c, r, false, false, "the popup closed");
                    self.view_screen(ui, r, "the popup closed");
                    self.finish(ui);
                }
            }
            _ => {}
        }
    }

    pub fn t_viewcpu(&mut self, ui: &mut Ui) {
        let r = view_rect(ui.fb().w, ui.fb().h);
        let o = ui.gpu_output(0);
        match self.pc {
            0 => {
                self.sc.views.one = Some(View3d::new(Kind::Small, 0.3));
                self.sc.views.ver = 1;
                self.next();
            }
            1 => {
                if self.after(800) {
                    let i = self.view_info(ui, 0, VIEW_ID);
                    out(&format!("  view {:?}, {:?}, {:?}", i.size, i.level, i.error));
                    self.ok(!o.wanted && !o.on && o.frames == 0, "jfgui_GPU_OUTPUT=0: the window on the CPU path");
                    self.ok(i.device && !i.output && i.level == Some(Level::Voll), &format!("the view on a device of its own ({:?})", i.level));
                    self.ok(!i.apart && i.frames >= 1 && i.reads >= i.frames, &format!("every frame read back: {} frames, {} read-backs", i.frames, i.reads));
                    let c = self.one();
                    self.view_check(ui, &c, r, false, true, "the CPU path");
                    let (x, y) = (r[0] + r[2] / 2, r[1] + r[3] / 2);
                    let fb = ui.fb();
                    let at = y as usize * fb.stride as usize + x as usize * 4;
                    let want = (fb.px[at] as u32 | (fb.px[at + 1] as u32) << 8 | (fb.px[at + 2] as u32) << 16) as i64;
                    let p = pixel_becomes(x, y, want);
                    self.ok(p == want, &format!("the window's DC shows the view ({p:06x}, the framebuffer {want:06x})"));
                    self.mark[0] = i.frames;
                    self.next();
                }
            }
            2 => {
                if self.view_turn(10) {
                    let i = self.view_info(ui, 0, VIEW_ID);
                    self.ok(i.frames >= self.mark[0] + 3 && i.reads >= i.frames, &format!("it turns: {} frames, {} read-backs in all", i.frames - self.mark[0], i.reads));
                    let c = self.one();
                    self.view_check(ui, &c, r, false, true, "the CPU path, turned");
                    self.finish(ui);
                }
            }
            _ => {}
        }
    }

    pub fn t_viewoff(&mut self, ui: &mut Ui) {
        let r = view_rect(ui.fb().w, ui.fb().h);
        match self.pc {
            0 => {
                self.sc.views.one = Some(View3d::new(Kind::Small, 0.3));
                self.sc.views.ver = 1;
                self.next();
            }
            1 => {
                if self.after(400) {
                    let i = self.view_info(ui, 0, VIEW_ID);
                    let c = self.one();
                    self.ok(!i.device && i.frames == 0 && c.borrow().attaches == 0, &format!("jfgui_GPU=0: no device, no frame ({:?})", i.error));
                    let (x, y) = (r[0] + r[2] / 2, r[1] + r[3] / 2);
                    let fb = ui.fb();
                    let at = y as usize * fb.stride as usize + x as usize * 4;
                    let p = fb.px[at] as u32 | (fb.px[at + 1] as u32) << 8 | (fb.px[at + 2] as u32) << 16;
                    self.ok(p == PAINT & 0xFF_FFFF, &format!("the content paints ({p:06x})"));
                    let w = pixel_becomes(x, y, (PAINT & 0xFF_FFFF) as i64);
                    self.ok(w == (PAINT & 0xFF_FFFF) as i64, &format!("the window's DC shows it ({w:06x})"));
                    self.next();
                }
            }
            2 => {
                if self.view_turn(5) {
                    let i = self.view_info(ui, 0, VIEW_ID);
                    self.ok(i.frames == 0, "a new version: still nothing drawn");
                    self.finish(ui);
                }
            }
            _ => {}
        }
    }

    pub fn t_viewbasis(&mut self, ui: &mut Ui) {
        let r = view_rect(ui.fb().w, ui.fb().h);
        match self.pc {
            0 => {
                if self.wait_on(ui, 15_000) == Some(true) {
                    self.sc.views.one = Some(View3d::new(Kind::Small, 0.3));
                    self.sc.views.ver = 1;
                    self.next();
                }
            }
            1 => {
                if self.after(150) {
                    let i = self.view_info(ui, 0, VIEW_ID);
                    self.ok(i.device && i.output && i.level == Some(Level::Basis), &format!("without AF_GPU_3D: the UI's device at level Basis ({:?})", i.level));
                    self.ok(i.apart, "drawn by the output's render pass");
                    let c = self.one();
                    self.view_check(ui, &c, r, true, true, "on the Basis device");
                    self.next();
                }
            }
            2 => {
                if self.view_turn(20) {
                    self.same_so_far(ui, "it turns on the Basis device");
                    self.next();
                }
            }
            3 => {
                if self.after(300) {
                    let c = self.one();
                    self.view_check(ui, &c, r, false, true, "at rest on the Basis device");
                    self.finish(ui);
                }
            }
            _ => {}
        }
    }
}

/// a test's views into its frame (draw_scene): the first in the window's middle (something over its corner), the
/// second below it
pub fn draw_views(v: &ViewSc, ui: &mut Ui) {
    let (w, h) = (ui.fb().w, ui.fb().h);
    if let Some(c) = &v.one {
        let r = view_rect(w, h);
        ui.next_rect_px(r[0], r[1], r[2], r[3]);
        let _ = ui.gpu_view(VIEW_ID, c, v.ver);
        if v.hud {
            ui.draw_color(0xC010_2030);
            ui.draw_rect(r[0] + 10, r[1] + 10, 210, 44);
            ui.draw_color(0xFFFF_FFFF);
            ui.draw_text(r[0] + 20, r[1] + 38, "über der Ansicht", (14u32 << 16) | FONT_REG);
        }
    }
    if let Some(c) = &v.two {
        let r = view2_rect(w, h);
        ui.next_rect_px(r[0], r[1], r[2], r[3]);
        let _ = ui.gpu_view(VIEW_ID2, c, v.ver2);
    }
}

// ---------------------------------------------------------------- the bench

struct ViewBench {
    content: Rc<RefCell<View3d>>,
    ver: u32,
    n: usize,
    idle: bool,
    started: u64,
    /// at the first measured frame: counters (frames, render us, present us, draw us, read us, reads, view frames)
    base: Option<[u64; 7]>,
    t_base: u64,
    done: bool,
    /// per measured view frame: the time since the last one (us)
    gaps: Vec<u64>,
    last: u64,
    seen: u64,
    /// (G5e2) the frames the output presented through the GPU at the first measured frame
    gpu_frames0: u64,
    /// (G5e2) the animation's timer (ms; --timer, 1: as fast as the loop takes it)
    timer: u32,
}

impl App for ViewBench {
    fn frame(&mut self, ui: &mut Ui) {
        let (w, h) = (ui.fb().w, ui.fb().h);
        ui.draw_color(0xFF20_2428);
        ui.draw_rect(0, 0, w, h);
        // (the title in cells of its own: nothing over the view, which starts on the second row of cells)
        ui.next_rect_px(40, 64, w - 80, h - 100);
        let _ = ui.gpu_view(VIEW_ID, &self.content, self.ver);
        ui.draw_color(0xFFE0_E4E8);
        ui.draw_text(48, 22, "uigputest --bench view", (14u32 << 16) | FONT_REG);
        if self.done {
            return;
        }
        if self.started == 0 {
            self.started = plat::time_us();
        }
        let o = ui.gpu_output(0);
        let i = ui.gpu_view_info(0, VIEW_ID).unwrap_or_default();
        let now = plat::time_us();
        // (the device and the first frames first: 3 s, or 1.5 s without an output)
        // (G5e2: an output that never comes, a GPU that cannot show this window: as without one)
        let warm = if o.wanted && !o.never { o.on && now > self.started + 3_000_000 } else { now > self.started + 1_500_000 };
        if !warm || i.frames == 0 {
            self.content.borrow_mut().angle += 0.01;
            self.ver += 1;
            ui.timer(1, self.timer); // (the next pass: an animation's timer, as fast as the loop takes it)
            return;
        }
        let counters = [o.frames + o.cpu_frames, o.render_us, o.present_us, i.draw_us, i.read_us, i.reads, i.frames];
        let Some(base) = self.base else {
            self.base = Some(counters);
            self.t_base = now;
            self.last = now;
            self.seen = i.frames;
            self.gpu_frames0 = o.frames;
            self.content.borrow_mut().gpu_ms.clear();
            if self.idle {
                self.started = plat::win_test_usage().0; // (now: the CPU time at the start)
                ui.timer(1, 5100); // (one look after 5 s, nothing before)
            } else {
                ui.timer(1, self.timer);
            }
            return;
        };
        if self.idle {
            // nothing changes for 5 s: what the window costs at rest
            if now >= self.t_base + 5_000_000 {
                let cpu = plat::win_test_usage().0 - self.started;
                let (_, ws, private) = plat::win_test_usage();
                out(&format!("idle 5 s: {} frames presented, {} of the view, {} read-backs, CPU {:.1} ms; reserved {:.1} MiB, working set {:.1} MiB, private {:.1} MiB",
                             counters[0] - base[0], counters[6] - base[6], counters[5] - base[5], cpu as f64 / 1000.0, o.reserved as f64 / 1048576.0,
                             ws as f64 / 1048576.0, private as f64 / 1048576.0));
                self.done = true;
                ui.quit();
            }
            return;
        }
        if i.frames != self.seen {
            self.seen = i.frames;
            self.gaps.push(now - self.last);
            self.last = now;
        }
        if (i.frames - base[6]) as usize >= self.n {
            let f = (i.frames - base[6]).max(1) as f64;
            let per = |k: usize| (counters[k] - base[k]) as f64 / f;
            self.gaps.sort_unstable();
            let med = self.gaps.get(self.gaps.len() / 2).copied().unwrap_or(0) as f64 / 1000.0;
            let mut g = core::mem::take(&mut self.content.borrow_mut().gpu_ms);
            g.sort_by(|a, b| a.partial_cmp(b).unwrap_or(core::cmp::Ordering::Equal));
            let gpu = g.get(g.len() / 2).copied().unwrap_or(0.0);
            let (_, ws, private) = plat::win_test_usage();
            let mode = if !o.wanted { "CPU path (jfgui_GPU_OUTPUT=0)" } else if i.apart { "composed by the output" } else { "read back every frame (jfgui_GPU_VIEWS=0)" };
            out(&format!("view bench, {mode}, {:?}, {} x {}: {} frames in {:.0} ms; per frame: interval {med:.3} ms (median), render {:.1} us, present {:.1} us, the view's draw {:.1} us, \
                          read-back {:.1} us ({:.2} per frame), render3d GPU {gpu:.3} ms (median); {} frames presented ({} through the GPU, on {}, pending {}, switches {}, \
                          leaves {}); reserved {:.1} MiB, working set {:.1} MiB, private {:.1} MiB",
                         i.level, i.size.0, i.size.1, f as u64, (now - self.t_base) as f64 / 1000.0, per(1), per(2), per(3), per(4), per(5), counters[0] - base[0],
                         o.frames - self.gpu_frames0, o.on, o.pending, o.switches, o.leaves, o.reserved as f64 / 1048576.0, ws as f64 / 1048576.0, private as f64 / 1048576.0));
            self.done = true;
            ui.quit();
            return;
        }
        self.content.borrow_mut().angle += 0.01;
        self.ver += 1;
        ui.timer(1, self.timer);
    }

    fn command(&mut self, ui: &mut Ui, id: u32, _arg: u32) -> bool {
        if id == UIC_TIMER {
            ui.invalidate();
            return true;
        }
        false
    }
}

/// render3d alone, offscreen: the same scene and target size, a frame recorded, submitted (and read back), waited for
fn offscreen_bench(n: usize, read: bool) -> i32 {
    let Ok(inst) = gpu::Instance::new() else {
        out("SKIP: no GPU");
        return 0;
    };
    let Ok(a) = inst.adapter() else {
        out("SKIP: no adapter");
        return 0;
    };
    let Ok(dev) = a.create_device(&gpu::DeviceDesc::DEFAULT) else {
        out("FAIL device");
        return 1;
    };
    let c = View3d::new(Kind::Gallery, 0.0);
    c.borrow_mut().stats = true;
    if c.borrow_mut().attach(&dev).is_err() {
        out("FAIL attach");
        return 1;
    }
    let (w, h) = (1200u32, 700u32);
    let Ok(t) = dev.create_texture(&gpu::TextureDesc::d2(w, h, GPU_VIEW_FORMAT, gpu::TextureUsage::COLOR_TARGET | gpu::TextureUsage::SAMPLED | gpu::TextureUsage::COPY_SRC)) else { return 1 };
    let stride = (w * 4 + 255) & !255;
    let Ok(rb) = dev.create_buffer(&gpu::BufferDesc { size: stride as u64 * h as u64, usage: gpu::BufferUsage::NONE, memory: gpu::MemoryKind::Readback }) else { return 1 };
    let mut raw = vec![0u8; (stride * h) as usize];
    let mut img = vec![0u8; (w * h * 4) as usize];
    let (mut cpu, mut reads, mut gaps) = (Vec::new(), Vec::new(), Vec::new());
    let mut fresh = true;
    let mut last = plat::time_us();
    for k in 0..n + 60 {
        let t0 = plat::time_us();
        let id = {
            let mut cb = c.borrow_mut();
            cb.angle += 0.01;
            let Ok(mut e) = dev.encoder() else { return 1 };
            let target = ColorAttachment { view: t.view(), load: Load::Clear, clear: [0.0; 4], store: true,
                                           before: if fresh { gpu::Layout::Undefined } else { gpu::Layout::Sampled }, after: gpu::Layout::Sampled };
            if cb.draw(&mut e, target, (w, h)).is_err() {
                out("FAIL draw");
                return 1;
            }
            let Ok(cmd) = e.finish() else { return 1 };
            let Ok(id) = dev.queue().submit([cmd]) else { return 1 };
            cb.submitted(id);
            id
        };
        fresh = false;
        let t1 = plat::time_us();
        if read {
            // what a view on the CPU path pays per frame: copy, wait, rows into the framebuffer
            let Ok(mut e) = dev.encoder() else { return 1 };
            e.texture_barrier(&t, gpu::Layout::Sampled, gpu::Layout::CopySrc);
            e.copy_texture_to_buffer(gpu::TextureRegion::mip(&t, 0), &rb, gpu::DataLayout { offset: 0, bytes_per_row: stride, rows_per_image: 0 });
            e.texture_barrier(&t, gpu::Layout::CopySrc, gpu::Layout::Sampled);
            e.barrier(gpu::Access::COPY_WRITE, gpu::Access::HOST_READ);
            let Ok(cmd) = e.finish() else { return 1 };
            let Ok(id) = dev.queue().submit([cmd]) else { return 1 };
            let _ = dev.wait(id, 10_000_000_000);
            let _ = rb.read(0, &mut raw);
            for y in 0..h as usize {
                img[y * w as usize * 4..][..w as usize * 4].copy_from_slice(&raw[y * stride as usize..][..w as usize * 4]);
            }
        } else {
            let _ = dev.wait(id, 10_000_000_000);
        }
        let t2 = plat::time_us();
        if k >= 60 {
            cpu.push(t1 - t0);
            reads.push(t2 - t1);
            gaps.push(t2 - last);
        }
        last = t2;
    }
    let med = |v: &mut Vec<u64>| {
        v.sort_unstable();
        v.get(v.len() / 2).copied().unwrap_or(0) as f64 / 1000.0
    };
    let mut g = core::mem::take(&mut c.borrow_mut().gpu_ms);
    g.sort_by(|a, b| a.partial_cmp(b).unwrap_or(core::cmp::Ordering::Equal));
    let gpu = g.get(g.len() / 2).copied().unwrap_or(0.0);
    out(&format!("view bench, render3d offscreen{}, {:?}, {w} x {h}: {n} frames; per frame (medians): interval {:.3} ms, record and submit {:.3} ms, {} {:.3} ms, render3d GPU {gpu:.3} ms; reserved {:.1} MiB",
                 if read { " with read-back" } else { "" }, dev.level(), med(&mut gaps), med(&mut cpu), if read { "read-back (copy, wait, rows)" } else { "wait" }, med(&mut reads),
                 dev.memory_reserved() as f64 / 1048576.0));
    c.borrow_mut().detach();
    0
}

pub fn view_bench(n: usize) -> i32 {
    let args = plat::args();
    let has = |a: &str| args.iter().any(|x| x == a);
    if has("--offscreen") {
        return offscreen_bench(n, has("--read"));
    }
    let content = View3d::new(Kind::Gallery, 0.0);
    content.borrow_mut().stats = true;
    let mut app = ViewBench { content, ver: 1, n, idle: has("--idle"), started: 0, base: None, t_base: 0, done: false, gaps: Vec::new(), last: 0, seen: 0, gpu_frames0: 0,
                             timer: plat::arg_value("--timer").and_then(|v| v.parse().ok()).filter(|v: &u32| *v > 0).unwrap_or(1) };
    let flags = if has("--basis") { AF_RESIZABLE } else { AF_RESIZABLE | AF_GPU_3D };
    let d = AppDesc { w: 1280, h: 800, flags, ..crate::DESC };
    ui::run_window(&d, &mut app, &style::STYLE)
}
