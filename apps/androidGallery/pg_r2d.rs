// "2D-Zeichnung" (G6e3): lib/render2d (docs/RENDER2D.md) in a view between the widgets: render2dtest's town plan
// (cad.rs) to move with a finger (one drags, two pinch) or the mouse (drag, wheel), the buttons zoom.  Its colours are
// sRGB codes as the view's target takes them.  Without a GPU the view paints the same scene through the CPU path
// (Scene::draw_cpu).
//
// "Prüfen" (and jfgui_GPU_R2DCHECK=1 by itself: the line "r2dcheck: done: N checks, M failed" at the end) draws on the
// view's device what render2dtest's cpu test draws on a desktop's: every image of render2dtest --dump (gallery::shots, 91:
// strokes, dashes, arcs, curves, fills, hatches, blocks, transparency, text, highlight, overlay) on the GPU against the
// CPU path of the same scene on the same device, per category within the cpu test's bounds (gallery::cpu_bounds), and
// the CPU path's bytes against those of every other target (gallery::CPU_HASH); the drawing at 1e7 against at the origin
// and lines, rings and dash ends at 1e-6 units per pixel (the f32 pairs, judged by the code of the coords and phase
// tests); two drawings of ezdxf from apps/render2dtest/dxf through render2d::dxf, GPU against CPU within the dxf test's
// bounds.  Every line also goes to the log ("r2dcheck: ...", logcat tag jfgui): tests/android_r2d.py reads them.
//
// "Messen" (jfgui_GPU_R2DBENCH=1 by itself) measures the town plan of 1 million records at 1920 x 1080 offscreen, with
// and without its text, at the overview, a district and the working view: per frame the GPU's timestamps (where the
// device has them), the wall time from submitting to done, prepare on the CPU; 3 frames to warm up, then 20, the median
// ("r2dbench: ...").  Both run on the view's device (the UI's, at its level), in steps on the page's timer.

use alloc::boxed::Box;
use alloc::format;
use alloc::rc::Rc;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::{Cell, RefCell};

use gpu::{ColorAttachment, CommandEncoder, Device, Format, GpuError, Layout, Load, SubmitId, Texture, TextureDesc, TextureUsage};
use render2d::dxf as rdxf;
use render2d::{Camera, Renderer, RendererDesc, Scene};
use ui::*;
use ui::i18n::t;

use crate::cad;
use crate::gallery::{self as gal, Off, BG};

const ID_VIEW: u32 = 9200;
const ID_CHECK: u32 = 9201;
const ID_BENCH: u32 = 9202;
const ID_IN: u32 = 9203;
const ID_OUT: u32 = 9204;
const ID_WHOLE: u32 = 9205;
/// the page's timer: the check's and the measurement's steps
pub const TIMER: u32 = 4310;
/// the view's background (sRGB codes)
const PAPER: u32 = 0xFFF4_F1EA;
/// the view's drawing: the town plan with this many records (the measurement's has 1 million).  Small, because without a
/// device (before the first frame, a lost one) the view paints it on the CPU, each time a part of it is drawn, in the
/// UI's thread: 150 000 records kept the emulator's UI busy for seconds (an ANR)
const VIEW_RECORDS: u32 = 40_000;

fn say(s: &str) {
    plat::write_stdout(format!("r2dcheck: {s}\n").as_bytes());
}

fn bench_say(s: &str) {
    plat::write_stdout(format!("r2dbench: {s}\n").as_bytes());
}

fn clear_of(c: u32) -> [f32; 4] {
    let k = |s: u32| ((c >> s) & 255) as f32 / 255.0;
    [k(16), k(8), k(0), 1.0]
}

fn median(v: &mut [f64]) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(core::cmp::Ordering::Equal));
    v[v.len() / 2]
}

/// render2d in the view: the scene (made without a device; on the device lib/ui hands over while there is one)
pub struct Content {
    pub cam: Option<Camera>,
    pub size: (u32, u32),
    pub dev: Option<Device>,
    r: Option<Renderer>,
    s: RefCell<Scene>,
    plan: Option<cad::Cad>,
    pub frames: u64,
    pub paints: Cell<u64>,
}

impl Content {
    fn new() -> Content {
        Content { cam: None, size: (0, 0), dev: None, r: None, s: RefCell::new(Scene::new_cpu()), plan: None, frames: 0, paints: Cell::new(0) }
    }

    /// the town plan, made when the page is first shown (not at the app's start)
    fn plan(&mut self) {
        if self.plan.is_none() {
            self.plan = cad::build(self.s.get_mut(), VIEW_RECORDS, 5, true, cad::Mode::Text).ok();
        }
    }

    /// the whole plan in the view's size
    fn whole(&self) -> Option<Camera> {
        let p = self.plan.as_ref()?;
        (self.size.0 > 0 && self.size.1 > 0).then(|| cad::views(p, self.size)[0].cam)
    }

    /// the camera the view shows (the whole plan until something moved it)
    fn camera(&mut self) -> Camera {
        if self.cam.is_none() {
            self.cam = self.whole();
        }
        self.cam.unwrap_or(Camera::new([0.0, 0.0], 1.0))
    }
}

impl GpuContent for Content {
    fn attach(&mut self, dev: &Device) -> gpu::Result<()> {
        let r = Renderer::new(dev, &RendererDesc::new(GPU_VIEW_FORMAT))?;
        self.s.get_mut().attach(&r)?;
        (self.r, self.dev) = (Some(r), Some(dev.clone()));
        Ok(())
    }

    fn detach(&mut self) {
        self.s.get_mut().detach();
        (self.r, self.dev) = (None, None);
    }

    fn draw<'a>(&'a mut self, e: &mut CommandEncoder<'a>, target: ColorAttachment<'a>, size: (u32, u32)) -> gpu::Result<()> {
        self.size = size;
        let cam = self.camera();
        {
            let (Some(r), s) = (self.r.as_mut(), self.s.get_mut()) else { return Err(GpuError::Invalid("not attached")) };
            s.set_camera(&cam)?;
            s.prepare(r, size)?;
        }
        self.frames += 1;
        let me: &'a mut Content = self;
        let (Some(r), s) = (me.r.as_ref(), me.s.get_mut()) else { return Err(GpuError::Invalid("not attached")) };
        let s: &'a Scene = s;
        r.record(e, s, ColorAttachment { load: Load::Clear, clear: clear_of(PAPER), ..target })
    }

    fn submitted(&mut self, id: SubmitId) {
        self.s.get_mut().submitted(id);
    }

    /// without a device: the same scene through the CPU path
    fn paint(&self, g: &mut Gfx, r: [i32; 4]) {
        g.dc.set_color(PAPER);
        g.dc.fill(g.sf, r[0], r[1], r[2], r[3]);
        let Ok(mut s) = self.s.try_borrow_mut() else { return };
        let cam = self.cam.or_else(|| self.plan.as_ref().map(|p| cad::views(p, (r[2].max(1) as u32, r[3].max(1) as u32))[0].cam));
        if let Some(c) = cam {
            if s.set_camera(&c).is_ok() {
                s.draw_cpu(g.dc, g.sf, r);
                self.paints.set(self.paints.get() + 1);
            }
        }
    }

    fn paints(&self) -> bool {
        true
    }
}

// ---------------------------------------------------------------- the check

/// one category's sums: images, pixels, beyond 2 codes, the largest difference, the ink of the GPU and the CPU path
#[derive(Default)]
struct Sum {
    name: String,
    images: u32,
    pixels: u64,
    over: u64,
    max: u8,
    ink: [f64; 2],
}

/// the GPU against the CPU path: pixels beyond 2 codes, the largest difference, both inks (refimg::compare's measure)
fn compare(gpu: &[u8], cpu: &[u8], bg: u32) -> (u64, u8, [f64; 2]) {
    let bgc = [(bg & 255) as i32, ((bg >> 8) & 255) as i32, ((bg >> 16) & 255) as i32];
    let (mut over, mut max, mut ink) = (0u64, 0u8, [0.0f64; 2]);
    for (p, q) in gpu.chunks_exact(4).zip(cpu.chunks_exact(4)) {
        let mut d = 0u8;
        for k in 0..3 {
            d = d.max(p[k].abs_diff(q[k]));
            ink[0] += (p[k] as i32 - bgc[k]).abs() as f64;
            ink[1] += (q[k] as i32 - bgc[k]).abs() as f64;
        }
        over += (d > 2) as u64;
        max = max.max(d);
    }
    (over, max, ink)
}

#[derive(Clone, Copy)]
enum Job {
    Shot(usize),
    /// the drawing (0 lines, 1 arcs and dashes, 2 curves, fills, hatches) at the origin (None) or at FAR_POINTS[k]
    Far(u8, Option<usize>),
    /// the deep zoom: 0 lines, 1 rings; its pan
    Deep(u8, u32),
    Phase(u32, u32),
    Dxf(usize),
}

static DXF_HATCH: &[u8] = include_bytes!("../render2dtest/dxf/clockwise_arcs_hatch.dxf");
static DXF_MTEXT: &[u8] = include_bytes!("../render2dtest/dxf/mtext_complex_inline_codes.dxf");

/// the repository's regular fonts for every family of a drawing (render2dtest's dxf test has the bold ones too: here both
/// paths draw the same text, which is what is compared)
fn font(f: rdxf::Family) -> Option<&'static [u8]> {
    use rdxf::Family;
    Some(match f {
        Family::Serif | Family::SerifBold => gal::SERIF,
        Family::Mono | Family::MonoBold => gal::MONO,
        _ => gal::SANS,
    })
}

static FONTS: fn(rdxf::Family) -> Option<&'static [u8]> = font;

struct Check {
    dev: Device,
    r: Renderer,
    jobs: Vec<Job>,
    next: usize,
    shots: Vec<gal::Shot>,
    sums: Vec<Sum>,
    hash: u64,
    origin: [Option<Vec<u8>>; 3],
    far: Vec<(usize, u8, Vec<u8>)>,
    deep: [gal::DeepStats; 2],
    phase: Vec<(f64, u32, &'static str)>,
    checks: u32,
    failed: u32,
    started: u64,
}

impl Check {
    fn new(dev: Device) -> gpu::Result<Check> {
        let r = Renderer::new(&dev, &RendererDesc::new(Format::Bgra8Unorm))?;
        let shots = gal::shots();
        let mut jobs: Vec<Job> = (0..shots.len()).map(Job::Shot).collect();
        for which in 0..3u8 {
            jobs.push(Job::Far(which, None));
            for k in 0..gal::FAR_POINTS.len() {
                jobs.push(Job::Far(which, Some(k)));
            }
        }
        for which in 0..2u8 {
            for step in 0..gal::DEEP_STEPS {
                jobs.push(Job::Deep(which, step));
            }
        }
        for k in 0..gal::PHASE_CASES {
            for step in 0..24 {
                jobs.push(Job::Phase(k, step));
            }
        }
        jobs.push(Job::Dxf(0));
        jobs.push(Job::Dxf(1));
        let phase = (0..gal::PHASE_CASES).map(|k| (0.0, 0, gal::phase_case(k, 0).name)).collect();
        Ok(Check { dev, r, jobs, next: 0, shots, sums: Vec::new(), hash: 0xcbf2_9ce4_8422_2325, origin: [None, None, None], far: Vec::new(),
                   deep: [gal::DeepStats::default(), gal::DeepStats::default()], phase, checks: 0, failed: 0, started: plat::time_us() })
    }

    fn yes(&mut self, ok: bool, what: &str) {
        self.checks += 1;
        self.failed += !ok as u32;
        say(&format!("{} {what}", if ok { "ok  " } else { "FAIL" }));
    }

    /// a scene's image on the GPU (waited for)
    fn gpu(&mut self, s: &mut Scene, off: &mut Off) -> gpu::Result<Vec<u8>> {
        let id = off.submit(&mut self.r, s)?;
        if !self.dev.wait(id, 10_000_000_000)? {
            return Err(GpuError::Timeout);
        }
        off.read()
    }

    /// one job -> an error ends the check
    fn job(&mut self, j: Job) -> gpu::Result<()> {
        match j {
            Job::Shot(k) => {
                let (name, w, h, cam) = {
                    let sh = &self.shots[k];
                    (sh.name.clone(), sh.w, sh.h, sh.cam)
                };
                let mut s = Scene::new(&self.r)?;
                s.set_camera(&cam)?;
                self.shots[k].build(&mut s)?;
                let mut off = Off::new(&self.dev, w, h)?;
                let g = self.gpu(&mut s, &mut off)?;
                let c = gal::cpu_image(&mut s, w, h, BG);
                self.hash = gal::fnv(self.hash, &c);
                let (over, max, ink) = compare(&g, &c, BG);
                let cat = gal::category(&name);
                let i = match self.sums.iter().position(|x| x.name == cat) {
                    Some(i) => i,
                    None => {
                        self.sums.push(Sum { name: cat.into(), ..Sum::default() });
                        self.sums.len() - 1
                    }
                };
                let su = &mut self.sums[i];
                su.images += 1;
                su.pixels += (w * h) as u64;
                su.over += over;
                su.max = su.max.max(max);
                su.ink[0] += ink[0];
                su.ink[1] += ink[1];
            }
            Job::Far(which, at) => {
                let p = at.map_or([0.0, 0.0], |k| gal::FAR_POINTS[k]);
                let mut s = Scene::new(&self.r)?;
                s.set_camera(&gal::sample_camera(p))?;
                gal::build(&mut s, &match which {
                    0 => gal::sample(p),
                    1 => gal::arc_sample(p),
                    _ => gal::curve_sample(p),
                })?;
                let mut off = Off::new(&self.dev, gal::FAR_SIZE.0, gal::FAR_SIZE.1)?;
                let img = self.gpu(&mut s, &mut off)?;
                match at {
                    None => self.origin[which as usize] = Some(img),
                    Some(k) => self.far.push((k, which, img)),
                }
            }
            Job::Deep(which, step) => {
                let mut s = Scene::new(&self.r)?;
                let c = gal::deep_camera(step);
                s.set_camera(&c)?;
                gal::build(&mut s, &if which == 0 { gal::deep_drawing() } else { gal::deep_arc_drawing() })?;
                let mut off = Off::new(&self.dev, gal::DEEP_SIZE.0, gal::DEEP_SIZE.1)?;
                off.bg = 0xFF00_0000;
                let img = self.gpu(&mut s, &mut off)?;
                if which == 0 {
                    gal::deep_judge(&img, &c, &mut self.deep[0]);
                } else {
                    gal::deep_arc_judge(&img, &c, &mut self.deep[1]);
                }
            }
            Job::Phase(k, step) => {
                let case = gal::phase_case(k, step);
                let mut s = Scene::new(&self.r)?;
                gal::build(&mut s, &case.d)?;
                s.set_camera(&case.cam)?;
                let mut off = Off::new(&self.dev, gal::PHASE_SIZE.0, gal::PHASE_SIZE.1)?;
                off.bg = 0xFF00_0000;
                let img = self.gpu(&mut s, &mut off)?;
                let e = &mut self.phase[k as usize];
                gal::phase_judge(&img, &case, k, &mut e.0, &mut e.1);
            }
            Job::Dxf(k) => {
                let (name, bytes, text) = if k == 0 { ("ezdxf clockwise_arcs_hatch.dxf", DXF_HATCH, false) } else { ("ezdxf mtext_complex_inline_codes.dxf", DXF_MTEXT, true) };
                let mut o = rdxf::Options::new();
                o.background = BG | 0xFF00_0000;
                o.fonts = Some(&FONTS);
                let mut s = Scene::new(&self.r)?;
                let l = match rdxf::load(&mut s, bytes, &o) {
                    Ok(l) => l,
                    Err(e) => {
                        self.yes(false, &format!("{name}: {e}"));
                        return Ok(());
                    }
                };
                let Some(b) = l.extents else {
                    self.yes(false, &format!("{name}: no extents"));
                    return Ok(());
                };
                // (800 x 600 as in render2dtest's dxf test, whose bounds these are: dxftest.rs SHARE, TEXT_SHARE)
                let (w, h) = (800u32, 600u32);
                let (bw, bh) = ((b[2] - b[0]).max(1e-9), (b[3] - b[1]).max(1e-9));
                s.set_camera(&Camera::new([(b[0] + b[2]) / 2.0, (b[1] + b[3]) / 2.0], (bw / w as f64).max(bh / h as f64) * 1.08))?;
                let mut off = Off::new(&self.dev, w, h)?;
                let g = self.gpu(&mut s, &mut off)?;
                let c = gal::cpu_image(&mut s, w, h, BG);
                let (over, max, ink) = compare(&g, &c, BG);
                let (share, most, inkl) = if text { (0.04, 255u8, 8.0) } else { (0.012, 210u8, 1.0) };
                let px = (w * h) as f64;
                let d = 100.0 * (ink[0] - ink[1]) / ink[1].max(1.0);
                self.yes(over as f64 <= share * px && max <= most && d.abs() <= inkl,
                         &format!("{name}: the GPU against the CPU path: {over} pixels beyond 2 codes ({:.3} %), largest {max}, ink {d:+.3} % (at most {:.1} %, {most}, {inkl} %)",
                                  100.0 * over as f64 / px, 100.0 * share));
            }
        }
        Ok(())
    }

    /// the verdicts once every job is done
    fn finish(&mut self) {
        let sums = core::mem::take(&mut self.sums);
        let (mut over, mut px) = (0u64, 0u64);
        for su in &sums {
            let (share, most, inkl) = gal::cpu_bounds(&su.name);
            let ink = 100.0 * (su.ink[0] - su.ink[1]) / su.ink[1].max(1.0);
            over += su.over;
            px += su.pixels;
            self.yes((su.over as f64) <= share * su.pixels as f64 && su.max <= most && ink.abs() <= inkl,
                     &format!("{:<13} {:>2} images: {:>6} of {:>8} pixels beyond 2 codes, largest {:>3}, ink GPU against CPU {:+.3} % (at most {:.1} %, {most}, {inkl} %)", su.name, su.images,
                              su.over, su.pixels, su.max, ink, 100.0 * share));
        }
        say(&format!("all: {over} of {px} pixels beyond 2 codes ({:.3} %)", 100.0 * over as f64 / px.max(1) as f64));
        let h = self.hash;
        self.yes(h == gal::CPU_HASH, &format!("the CPU path's {} images: FNV-1a {h:016x} (every other target: {:016x})", self.shots.len(), gal::CPU_HASH));
        if let (Some(a), Some(b), Some(c)) = (self.origin[0].take(), self.origin[1].take(), self.origin[2].take()) {
            let (w, ht) = gal::FAR_SIZE;
            for (k, which, img) in core::mem::take(&mut self.far) {
                let (n, m) = gal::differ(&img, [&a, &b, &c][which as usize]);
                let p = gal::FAR_POINTS[k];
                let what = ["the drawing", "arcs, circles and dash patterns", "curves, a fill with a hole and a hatched fill"][which as usize];
                self.yes(m <= 1 && n <= (w * ht) / 1000, &format!("{what} at ({}, {}) against at the origin: {n} pixels differ, largest {m}", p[0], p[1]));
            }
        }
        let (fine, text) = (self.deep[0].fine(), self.deep[0].text());
        self.yes(fine, &text);
        let (fine, text) = (self.deep[1].fine_arcs(), self.deep[1].arc_text());
        self.yes(fine, &text);
        for (worst, checks, name) in core::mem::take(&mut self.phase) {
            self.yes(checks >= 100 && worst <= 1.0 / 16.0,
                     &format!("dashes at 1e-6 units per pixel, {name}: {checks} pixel lines over 24 pans, the largest distance of a dash's end from where it belongs {worst:.4} px (1/16 = 0.0625)"));
        }
    }
}

// ---------------------------------------------------------------- the measurement

struct Bench {
    dev: Device,
    r: Renderer,
    s: Option<Scene>,
    plan: Option<cad::Cad>,
    target: Texture,
    used: bool,
    /// 0: with text, 1: without
    pass: usize,
    view: usize,
    frame: u32,
    gpu: Vec<f64>,
    wall: Vec<f64>,
    prep: Vec<f64>,
    visible: u64,
    stamps: bool,
}

const BENCH_SIZE: (u32, u32) = (1920, 1080);
const BENCH_WARM: u32 = 3;
const BENCH_FRAMES: u32 = 20;

impl Bench {
    fn new(dev: Device) -> gpu::Result<Bench> {
        let r = Renderer::new(&dev, &RendererDesc::new(Format::Bgra8Unorm))?;
        let target = dev.create_texture(&TextureDesc::d2(BENCH_SIZE.0, BENCH_SIZE.1, Format::Bgra8Unorm, TextureUsage::COLOR_TARGET))?;
        Ok(Bench { dev, r, s: None, plan: None, target, used: false, pass: 0, view: 0, frame: 0, gpu: Vec::new(), wall: Vec::new(), prep: Vec::new(), visible: 0, stamps: false })
    }

    /// one step: the scene of a pass, or one frame -> done
    fn step(&mut self) -> gpu::Result<bool> {
        if self.pass >= 2 {
            return Ok(true);
        }
        let label = if self.pass == 0 { "with text" } else { "without text" };
        let Some(mut s) = self.s.take() else {
            let mut s = Scene::new(&self.r)?;
            s.set_stats(true);
            let t0 = plat::time_us();
            let c = cad::build(&mut s, 1_000_000, 5, true, if self.pass == 0 { cad::Mode::Text } else { cad::Mode::NoText })?;
            bench_say(&format!("{label}: {} entities, {} records, {} texts; inserted in {:.0} ms (CPU)", c.entities, c.segments, c.texts, (plat::time_us() - t0) as f64 / 1000.0));
            (self.s, self.plan) = (Some(s), Some(c));
            return Ok(false);
        };
        let Some(plan) = self.plan.as_ref() else { return Ok(true) };
        let v = &cad::views(plan, BENCH_SIZE)[self.view];
        let mut cam = v.cam;
        cam.center[0] += self.frame as f64 * 0.37 * cam.scale;
        s.set_camera(&cam)?;
        let t0 = plat::time_us();
        s.prepare(&mut self.r, BENCH_SIZE)?;
        let prep = (plat::time_us() - t0) as f64 / 1000.0;
        let mut e = self.dev.encoder()?;
        let color = ColorAttachment { view: self.target.view(), load: Load::Clear, clear: clear_of(BG), store: true, before: if self.used { Layout::ColorTarget } else { Layout::Undefined },
                                      after: Layout::ColorTarget };
        self.r.record(&mut e, &s, color)?;
        let cb = e.finish()?;
        let t1 = plat::time_us();
        let id = self.dev.queue().submit([cb])?;
        s.submitted(id);
        self.used = true;
        if !self.dev.wait(id, 30_000_000_000)? {
            return Err(GpuError::Timeout);
        }
        let wall = (plat::time_us() - t1) as f64 / 1000.0;
        s.collect_stats();
        let st = s.take_stats();
        if self.frame >= BENCH_WARM {
            if let Some(f) = st.last() {
                self.visible = f.visible;
                if let Some(v) = f.stamps {
                    self.stamps = true;
                    self.gpu.push(v[3] - v[0]);
                }
            }
            self.wall.push(wall);
            self.prep.push(prep);
        }
        self.frame += 1;
        if self.frame == BENCH_WARM + BENCH_FRAMES {
            let g = if self.stamps { format!("{:.3} ms", median(&mut self.gpu)) } else { String::from("no timestamps") };
            bench_say(&format!("{label}, {} (scale {:.3}): GPU {g}; submit to done {:.3} ms (wall); prepare {:.3} ms (CPU); {} records visible (median of {BENCH_FRAMES})", v.name, v.cam.scale,
                               median(&mut self.wall), median(&mut self.prep), self.visible));
            (self.frame, self.gpu, self.wall, self.prep) = (0, Vec::new(), Vec::new(), Vec::new());
            self.view += 1;
            if self.view == 3 {
                (self.view, self.plan) = (0, None);
                self.pass += 1;
                let _ = self.dev.wait_idle();
                return Ok(self.pass >= 2);
            }
        }
        self.s = Some(s);
        Ok(false)
    }
}

// ---------------------------------------------------------------- the page

/// two fingers on the view: which (the host's ids), their midpoint (px of the window) and distance when this pair came
/// down, the camera then
#[derive(Clone, Copy)]
struct Pinch {
    ids: [u8; 2],
    mid: [f64; 2],
    dist: f64,
    cam: Camera,
}

pub struct View2d {
    content: Rc<RefCell<Content>>,
    ver: u32,
    rect: [i32; 4],
    drag: Option<(i32, i32)>,
    pinch: Option<Pinch>,
    check: Option<Box<Check>>,
    bench: Option<Box<Bench>>,
    /// jfgui_GPU_R2DCHECK / R2DBENCH: started by itself (once), waits for the view's device since then
    auto: u8,
    since: u64,
    result: String,
}

impl Default for View2d {
    fn default() -> Self {
        Self { content: Rc::new(RefCell::new(Content::new())), ver: 1, rect: [0; 4], drag: None, pinch: None, check: None, bench: None, auto: 0, since: 0, result: String::new() }
    }
}

impl View2d {
    pub fn page(&mut self, ui: &mut Ui) {
        if self.auto == 0 {
            self.auto = 1;
            let on = |k: &str| plat::env(k).is_some_and(|v| v == "1");
            if on("jfgui_GPU_R2DCHECK") || on("jfgui_GPU_R2DBENCH") {
                self.auto = if on("jfgui_GPU_R2DCHECK") { 2 } else { 3 };
                self.since = plat::time_us();
                ui.timer(TIMER, 50);
            }
        }
        self.content.borrow_mut().plan();
        ui.text(TS_BODY | TX_WRAP | TX_SECONDARY, t("Eine CAD-Zeichnung zwischen den Widgets: Ziehen verschiebt, zwei Finger zoomen, die GPU zeichnet sie (render2d)."));
        ui.next_size(SIZE_FILL, 360);
        let inp = ui.gpu_view(ID_VIEW, &self.content, self.ver).clone();
        ui.touch_take(TF_BOTH | TF_TWO);
        self.rect = inp.rect;
        let size = (inp.rect[2].max(1) as u32, inp.rect[3].max(1) as u32);
        let mut moved = false;
        // Two fingers whose first came down on the view zoom and pan together, however late the second one came.
        // ui.touch_two holds a second finger back once the first one's drag is older than 200 ms (meant for a stroke
        // that goes on), and a pinch whose fingers came down a little apart became a pan with one of them (found in the
        // user's hand check on the phone, G6e3).  So the pair comes from ui.touch_fingers, as TF_TWO allows; a new pair
        // (other ids) starts from the camera of that moment, the first finger's drag waits meanwhile
        let f = ui.touch_fingers();
        let r = inp.rect;
        let on_view = |x: i32, y: i32| x >= r[0] && y >= r[1] && x < r[0] + r[2] && y < r[1] + r[3];
        let pair = if f.len() >= 2 && on_view(f[0].down_x, f[0].down_y) { Some((f[0], f[1])) } else { None };
        if let Some((a, b)) = pair {
            let mid = [(a.x + b.x) as f64 / 2.0, (a.y + b.y) as f64 / 2.0];
            let (dx, dy) = ((a.x - b.x) as f64, (a.y - b.y) as f64);
            let dist = sqrt(dx * dx + dy * dy).max(1.0);
            let mut c = self.content.borrow_mut();
            c.size = size;
            if self.pinch.is_none_or(|p| p.ids != [a.id, b.id]) {
                self.pinch = Some(Pinch { ids: [a.id, b.id], mid, dist, cam: c.camera() });
            }
            if let Some(p) = self.pinch {
                let mut cam = p.cam;
                cam.zoom_at([p.mid[0] - r[0] as f64, p.mid[1] - r[1] as f64], (dist / p.dist).clamp(0.05, 20.0), size);
                cam.pan(mid[0] - p.mid[0], mid[1] - p.mid[1]);
                c.cam = Some(cam);
                moved = true;
            }
            self.drag = None;
        } else {
            self.pinch = None;
            if inp.ev & EV_DRAG != 0 || inp.btn != 0 {
                if let Some((x, y)) = self.drag.filter(|d| *d != (inp.x, inp.y)) {
                    let mut c = self.content.borrow_mut();
                    c.size = size;
                    let mut cam = c.camera();
                    cam.pan((inp.x - x) as f64, (inp.y - y) as f64);
                    c.cam = Some(cam);
                    moved = true;
                }
                self.drag = Some((inp.x, inp.y));
            } else {
                self.drag = None;
            }
        }
        if inp.wheel_y != 0 {
            self.zoom(Some([inp.x as f64 + 0.5, inp.y as f64 + 0.5]), pow2(inp.wheel_y as f64 / 480.0), size);
            moved = true;
        }
        ui.box_begin(0, BOX_H, 8);
        if ui.button(ID_IN, "+") & EV_ACT != 0 {
            self.zoom(None, 2.0, size);
            moved = true;
        }
        if ui.button(ID_OUT, "−") & EV_ACT != 0 {
            self.zoom(None, 0.5, size);
            moved = true;
        }
        if ui.button(ID_WHOLE, t("Ganz")) & EV_ACT != 0 {
            let mut c = self.content.borrow_mut();
            c.size = size;
            c.cam = c.whole();
            moved = true;
        }
        ui.box_end();
        if moved {
            self.ver = self.ver.wrapping_add(1);
            ui.invalidate();
        }
        ui.box_begin(0, BOX_H, 8);
        if ui.button(ID_CHECK, t("Prüfen")) & EV_ACT != 0 && self.check.is_none() && self.bench.is_none() {
            self.start(ui, true);
        }
        if ui.button(ID_BENCH, t("Messen")) & EV_ACT != 0 && self.check.is_none() && self.bench.is_none() {
            self.start(ui, false);
        }
        ui.box_end();
        let i = ui.gpu_view_info(0, ID_VIEW).unwrap_or_default();
        let c = self.content.borrow();
        let dev = if i.device { format!("{:?}", i.level.unwrap_or(gpu::Level::Basis)) } else { String::from(t("kein Gerät")) };
        let scale = c.cam.or_else(|| c.whole()).map_or(0.0, |k| k.scale);
        ui.text(TS_CAPTION | TX_WRAP | TX_SECONDARY, &format!("{dev}; {} {}, {} {}; 1 px = {scale:.3} m", c.frames, t("Bilder"), c.paints.get(), t("auf der CPU")));
        drop(c);
        if !self.result.is_empty() {
            ui.text(TS_BODY | TX_WRAP, &self.result);
        }
    }

    /// zoom by f at a point of the view (px; None: its middle)
    fn zoom(&mut self, at: Option<[f64; 2]>, f: f64, size: (u32, u32)) {
        let mut c = self.content.borrow_mut();
        c.size = size;
        let mut cam = c.camera();
        cam.zoom_at(at.unwrap_or([size.0 as f64 / 2.0, size.1 as f64 / 2.0]), f, size);
        c.cam = Some(cam);
    }

    /// the check or the measurement on the view's device
    fn start(&mut self, ui: &mut Ui, check: bool) {
        let Some(dev) = self.content.borrow().dev.clone() else {
            self.result = String::from(t("Kein GPU-Gerät: nichts zu prüfen oder zu messen."));
            if check {
                say("no device: the view paints through the CPU path");
                say("done: 0 checks, 0 failed (no device)");
            } else {
                bench_say("FAIL no device: nothing measured");
                bench_say("done");
            }
            return;
        };
        let level = dev.level();
        if check {
            match Check::new(dev) {
                Ok(c) => {
                    say(&format!("start: {} images, the far drawings, the deep zoom, the dash ends, two DXF drawings on the view's device ({level:?})", c.shots.len()));
                    self.check = Some(Box::new(c));
                    self.result = String::from(t("Prüfung läuft …"));
                }
                Err(e) => {
                    say(&format!("FAIL the renderer: {e}"));
                    say("done: 1 checks, 1 failed");
                }
            }
        } else {
            match Bench::new(dev) {
                Ok(b) => {
                    bench_say(&format!("start: the town plan of 1 million records at {} x {} on the view's device ({level:?})", BENCH_SIZE.0, BENCH_SIZE.1));
                    self.bench = Some(Box::new(b));
                    self.result = String::from(t("Messung läuft …"));
                }
                Err(e) => {
                    bench_say(&format!("FAIL the renderer: {e}"));
                    bench_say("done");
                }
            }
        }
        ui.timer(TIMER, 16);
    }

    /// the page's timer: the self-tests' start, the check's and the measurement's steps
    pub fn command(&mut self, ui: &mut Ui, id: u32, arg: u32) -> bool {
        if id != UIC_TIMER || arg != TIMER {
            return false;
        }
        if self.auto >= 2 {
            let ready = self.content.borrow().dev.is_some() && self.content.borrow().frames > 0;
            if ready || plat::time_us() > self.since + 20_000_000 {
                let check = self.auto == 2;
                self.auto = 1;
                self.start(ui, check);
            } else {
                ui.timer(TIMER, 50);
            }
            return true;
        }
        let t0 = plat::time_us();
        if let Some(mut c) = self.check.take() {
            let mut err = None;
            // (jobs for about 30 ms, then the UI's turn)
            while c.next < c.jobs.len() && plat::time_us() < t0 + 30_000 {
                let j = c.jobs[c.next];
                c.next += 1;
                if let Err(e) = c.job(j) {
                    err = Some(e);
                    break;
                }
            }
            if let Some(e) = err {
                c.yes(false, &format!("step {}: {e}", c.next));
                c.next = c.jobs.len();
            }
            if c.next >= c.jobs.len() {
                c.finish();
                let ms = (plat::time_us() - c.started) / 1000;
                say(&format!("done: {} checks, {} failed ({ms} ms)", c.checks, c.failed));
                self.result = format!("{} {}: {} / {}", t("Prüfung"), if c.failed == 0 { t("in Ordnung") } else { t("abweichend") }, c.checks - c.failed, c.checks);
                if plat::env("jfgui_GPU_R2DCHECK").is_some_and(|v| v == "1") {
                    ui.quit();
                }
            } else {
                self.check = Some(c);
                ui.timer(TIMER, 1);
            }
        }
        if let Some(mut b) = self.bench.take() {
            match b.step() {
                Ok(false) => {
                    self.bench = Some(b);
                    ui.timer(TIMER, 1);
                }
                Ok(true) => {
                    bench_say("done");
                    self.result = String::from(t("Messung fertig (Protokoll: r2dbench)"));
                    if plat::env("jfgui_GPU_R2DBENCH").is_some_and(|v| v == "1") {
                        ui.quit();
                    }
                }
                Err(e) => {
                    bench_say(&format!("FAIL {e}"));
                    bench_say("done");
                    self.result = format!("{e}");
                }
            }
        }
        ui.invalidate();
        true
    }
}

/// the square root for the fingers' distance (no libm): Newton from half the exponent
fn sqrt(v: f64) -> f64 {
    if v <= 0.0 || !v.is_finite() {
        return if v.is_finite() { 0.0 } else { v };
    }
    let mut x = f64::from_bits((v.to_bits() >> 1) + (1023u64 << 51));
    for _ in 0..6 {
        x = 0.5 * (x + v / x);
    }
    x
}

/// 2^x for the wheel's zoom (no libm): a notch zooms by 2^(1/4)
fn pow2(x: f64) -> f64 {
    let k = if x < 0.0 { (x - 1.0) as i64 } else { x as i64 };
    let r = (x - k as f64) * core::f64::consts::LN_2;
    let (mut term, mut sum) = (1.0, 1.0);
    for n in 1..20 {
        term *= r / n as f64;
        sum += term;
    }
    sum * f64::from_bits(((k + 1023).clamp(1, 2046) as u64) << 52)
}
