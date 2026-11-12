// web -- render2dtest in the browser (tests/engine_web.mjs reads the console; the app itself is the demo unless the
// address says ?gpuenv=jfgui_GPU_PROBE=test or =bench).  The renderer over WebGPU, from the same WGSL as on Vulkan.
// Nothing can wait here: every frame is submitted at once, the read-backs are looked at per tick.
//
//   test    the images of gallery::shots() go out as hex lines (the test compares them with the same images of the
//           Vulkan path on this machine), the coordinates (the same drawing at the origin and at 1e7, also arcs and
//           dashes; the deep zoom to 1e-6 units per pixel, lines and rings), the dash pattern's place far away (phase)
//           and the visibility are judged here, by the code of the Vulkan tests
//   bench   the CAD scene (cad.rs; jfgui_GPU_PROBE_N segments, 1 million by default) at 1920 x 1080, three frames ahead:
//           the wall clock from one frame's end to the next's.  WebGPU has no timestamps in lib/gpu: what a frame costs
//           here includes the browser's own work

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use gpu::*;
use render2d::{Camera, Renderer, RendererDesc, Scene};

use crate::cad;
use crate::gallery::*;

fn say(s: &str) {
    rt::os::write_stdout(format!("{s}\n").as_bytes());
}

fn hex_line(name: &str, bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2 + name.len() + 8);
    s.push_str(&format!("img {name} "));
    for b in bytes {
        s.push(HEX[(b >> 4) as usize] as char);
        s.push(HEX[(b & 15) as usize] as char);
    }
    s
}

enum Kind {
    /// sent as hex (and looked at when translucent)
    Send(String, Option<u32>),
    /// the drawing around a point: index into far, or the origin's (0 lines, 1 arcs and dashes, 2 curves, fills, hatches)
    Far(Option<usize>, u8),
    /// a frame of the deep zoom: lines, rings (arcs) or curves
    Deep(Camera, u8),
    /// a frame of the deep hatch: hatched or solid
    DeepHatch(Camera, bool),
    /// a frame of a phase case: the case, its pan
    Phase(u32, u32),
}

struct Job {
    off: Off,
    kind: Kind,
}

/// a visibility case on its way back
struct VisJob {
    s: Scene,
    cam: Camera,
    ents: Vec<VisEnt>,
    seed: u32,
    n: u32,
    spread: f64,
    lists: Buffer,
    vis: Buffer,
    layers: usize,
    /// (the images of `culled = not culled`)
    pair: Option<(Off, Off)>,
    _off: Off,
}

struct Test {
    jobs: Vec<Job>,
    scenes: Vec<Scene>,
    vis: Vec<VisJob>,
    origin: [Option<Vec<u8>>; 3],
    far: Vec<(usize, u8, Vec<u8>)>,
    deep: DeepStats,
    deep_arcs: DeepStats,
    deep_curves: DeepStats,
    /// (G6b2) the deep zoom's lines as an insert moved there and as one scaled by 1e5
    deep_ins: [DeepStats; 2],
    fine: Vec<Vec<[f64; 2]>>,
    /// the deep hatch: a hatched frame waiting for its solid one; the lines' and the outline's stats
    hatched: Option<Vec<u8>>,
    hatch_lines: DeepStats,
    hatch_edge: DeepStats,
    /// per phase case: the largest distance of a dash's end, the pixel lines looked at, its name
    phase: Vec<(f64, u32, &'static str)>,
    phase_done: bool,
}

struct Bench {
    r: Renderer,
    s: Scene,
    off: Texture,
    used: bool,
    size: (u32, u32),
    cads: [cad::Cad; 1],
    styled: bool,
    n: u32,
    view: usize,
    /// frames of this view submitted, done
    sent: u32,
    done: u32,
    flight: Vec<(SubmitId, u32)>,
    /// the moment the frame after the warm-up finished; the last frame's end
    t0: u64,
    cam: Camera,
    cpu_ms: Vec<f64>,
}

enum Mode {
    None,
    Test(Box<Test>),
    Bench(Box<Bench>),
}

struct Probe {
    step: u32,
    probe: String,
    inst: Option<Instance>,
    dev: Option<Device>,
    mode: Mode,
    ok: u32,
    bad: u32,
    t0: u64,
    limit_s: u64,
}

const BENCH_FRAMES: u32 = 30;
const WARM: u32 = 3;
const AHEAD: usize = 3;

impl Probe {
    fn yes(&mut self, ok: bool, what: &str) {
        if ok {
            self.ok += 1;
            say(&format!("ok   {what}"));
        } else {
            self.bad += 1;
            say(&format!("FAIL {what}"));
        }
    }

    fn end(&mut self) {
        say(&format!("render2dtest: {} ok, {} failed, {:.1} ms", self.ok, self.bad, (plat::time_us() - self.t0) as f64 / 1000.0));
        self.step = u32::MAX;
    }

    /// everything of the test submitted
    fn start_test(&mut self, dev: &Device) -> Result<()> {
        let mut r = Renderer::new(dev, &RendererDesc::new(Format::Bgra8Unorm))?;
        say(&format!("renderer: multi-draw {}, at most {} records", r.multi_draw(), r.max_records()));
        self.yes(!r.multi_draw(), "the browser's level Basis: one indirect draw per layer");
        let (dc, fine) = deep_curves();
        let mut t = Test { jobs: Vec::new(), scenes: Vec::new(), vis: Vec::new(), origin: [None, None, None], far: Vec::new(), deep: DeepStats::default(), deep_arcs: DeepStats::default(),
                           deep_curves: DeepStats::default(), fine, hatched: None, hatch_lines: DeepStats::default(), hatch_edge: DeepStats::default(), phase: Vec::new(), phase_done: false,
                           deep_ins: [DeepStats::default(); 2] };
        for shot in shots() {
            let mut off = Off::new(dev, shot.w, shot.h)?;
            let mut s = Scene::new(&r)?;
            s.set_camera(&shot.cam)?;
            shot.build(&mut s)?;
            off.submit(&mut r, &mut s)?;
            t.jobs.push(Job { off, kind: Kind::Send(shot.name, shot.alpha) });
            t.scenes.push(s);
        }
        // the same drawing at the origin and at +-1e7 (lines; arcs and dashes; curves, fills, hatches)
        for which in 0..3u8 {
            for k in 0..FAR_POINTS.len() + 1 {
                let at = if k == 0 { [0.0, 0.0] } else { FAR_POINTS[k - 1] };
                let mut off = Off::new(dev, FAR_SIZE.0, FAR_SIZE.1)?;
                let mut s = Scene::new(&r)?;
                s.set_camera(&sample_camera(at))?;
                build(&mut s, &match which {
                    0 => sample(at),
                    1 => arc_sample(at),
                    _ => curve_sample(at),
                })?;
                off.submit(&mut r, &mut s)?;
                t.jobs.push(Job { off, kind: Kind::Far(if k == 0 { None } else { Some(k - 1) }, which) });
                t.scenes.push(s);
            }
        }
        // the deep zoom: lines, rings, curves; the lines as a block moved there and scaled by 1e5 (G6b2)
        for which in 0..5u8 {
            let mut s = Scene::new(&r)?;
            s.set_camera(&deep_camera(0))?;
            match which {
                3 => build_blocks(&mut s, &deep_insert_drawing(1.0, 0.0, false)).map(|_| ())?,
                4 => build_blocks(&mut s, &deep_insert_drawing(1.0e5, -1.1, true)).map(|_| ())?,
                _ => build(&mut s, &match which {
                    0 => deep_drawing(),
                    1 => deep_arc_drawing(),
                    _ => dc.clone(),
                }).map(|_| ())?,
            }
            for step in 0..DEEP_STEPS {
                let c = deep_camera(step);
                s.set_camera(&c)?;
                let mut off = Off::new(dev, DEEP_SIZE.0, DEEP_SIZE.1)?;
                off.bg = 0xFF00_0000;
                off.submit(&mut r, &mut s)?;
                t.jobs.push(Job { off, kind: Kind::Deep(c, which) });
            }
            t.scenes.push(s);
        }
        // the deep hatch: hatched and solid, frame by frame
        let (mut sh, mut ss) = (Scene::new(&r)?, Scene::new(&r)?);
        sh.set_camera(&deep_camera(0))?;
        ss.set_camera(&deep_camera(0))?;
        build(&mut sh, &deep_hatch(false))?;
        build(&mut ss, &deep_hatch(true))?;
        for step in 0..DEEP_STEPS {
            let c = deep_camera(step);
            for (solid, s) in [(false, &mut sh), (true, &mut ss)] {
                s.set_camera(&c)?;
                let mut off = Off::new(dev, DEEP_SIZE.0, DEEP_SIZE.1)?;
                off.bg = 0xFF00_0000;
                off.submit(&mut r, s)?;
                t.jobs.push(Job { off, kind: Kind::DeepHatch(c, solid) });
            }
        }
        t.scenes.push(sh);
        t.scenes.push(ss);
        // the dash pattern's place far away: each case's 24 pans
        for k in 0..PHASE_CASES {
            let mut s = Scene::new(&r)?;
            build(&mut s, &phase_case(k, 0).d)?;
            for step in 0..24 {
                s.set_camera(&phase_case(k, step).cam)?;
                let mut off = Off::new(dev, PHASE_SIZE.0, PHASE_SIZE.1)?;
                off.bg = 0xFF00_0000;
                off.submit(&mut r, &mut s)?;
                t.jobs.push(Job { off, kind: Kind::Phase(k, step) });
            }
            t.phase.push((0.0, 0, phase_case(k, 0).name));
            t.scenes.push(s);
        }
        // the visibility: the lists read back and judged by the exact test on the CPU
        for (seed, n, spread) in VIS_CASES {
            let (mut s, cam, ents) = vis_scene(&r, seed, n, spread)?;
            let mut off = Off::new(dev, VIS_SIZE.0, VIS_SIZE.1)?;
            off.submit(&mut r, &mut s)?;
            let layers = s.drawn_layers().len();
            let rb = |size: u64| dev.create_buffer(&BufferDesc { size: size.max(16), usage: BufferUsage::NONE, memory: MemoryKind::Readback });
            let (lb, vb) = (rb(layers as u64 * 16)?, rb(s.visible_buffer().size())?);
            let mut e = dev.encoder()?;
            if layers > 0 {
                e.copy_buffer(s.lists_buffer(), 0, &lb, 0, layers as u64 * 16);
            }
            e.copy_buffer(s.visible_buffer(), 0, &vb, 0, s.visible_buffer().size());
            dev.queue().submit([e.finish()?])?;
            let mut pair = None;
            if seed == 1 {
                let (mut a, mut b) = (Off::new(dev, VIS_SIZE.0, VIS_SIZE.1)?, Off::new(dev, VIS_SIZE.0, VIS_SIZE.1)?);
                a.submit(&mut r, &mut s)?;
                s.set_culling(false);
                b.submit(&mut r, &mut s)?;
                pair = Some((a, b));
            }
            t.vis.push(VisJob { s, cam, ents, seed, n, spread, lists: lb, vis: vb, layers, pair, _off: off });
        }
        // (the renderer lives as long as the frames: held with the scenes' buffers by the browser)
        self.mode = Mode::Test(Box::new(t));
        Ok(())
    }

    fn poll_test(&mut self, t: &mut Test) -> Result<bool> {
        while let Some(j) = t.jobs.first() {
            let img = match j.off.read() {
                Err(GpuError::NotReady) => return Ok(false),
                r => r?,
            };
            match &j.kind {
                Kind::Send(name, alpha) => {
                    // long lines in pieces: consoles and the bridge have limits
                    let line: String = hex_line(name, &img);
                    for (i, piece) in line.as_bytes().chunks(16000).enumerate() {
                        let text = core::str::from_utf8(piece).unwrap_or("");
                        say(&format!("part {i} {text}"));
                    }
                    say("part end");
                    if let Some(a) = alpha {
                        let twice = blended_twice(&img, *a);
                        self.yes(twice == 0, &format!("{name}: no pixel beyond one blend of the colour: {twice}"));
                    }
                }
                Kind::Far(None, which) => t.origin[*which as usize] = Some(img),
                Kind::Far(Some(k), which) => t.far.push((*k, *which, img)),
                Kind::Deep(c, 0) => deep_judge(&img, c, &mut t.deep),
                Kind::Deep(c, 1) => deep_arc_judge(&img, c, &mut t.deep_arcs),
                Kind::Deep(c, 3) => deep_judge(&img, c, &mut t.deep_ins[0]),
                Kind::Deep(c, 4) => deep_judge(&img, c, &mut t.deep_ins[1]),
                Kind::Deep(c, _) => deep_curve_judge(&img, c, &t.fine, &mut t.deep_curves),
                Kind::DeepHatch(_, false) => t.hatched = Some(img),
                Kind::DeepHatch(c, true) => {
                    if let Some(a) = t.hatched.take() {
                        deep_hatch_judge(&a, &img, c, &mut t.hatch_lines, &mut t.hatch_edge);
                    }
                }
                Kind::Phase(k, step) => {
                    let case = phase_case(*k, *step);
                    let e = &mut t.phase[*k as usize];
                    phase_judge(&img, &case, *k, &mut e.0, &mut e.1);
                }
            }
            t.jobs.remove(0);
        }
        // the coordinates
        if let (Some(base), Some(base_arcs), Some(base_curves)) = (t.origin[0].take(), t.origin[1].take(), t.origin[2].take()) {
            let (w, h) = FAR_SIZE;
            for (k, which, img) in core::mem::take(&mut t.far) {
                let (n, m) = differ(&img, [&base, &base_arcs, &base_curves][which as usize]);
                let at = FAR_POINTS[k];
                let what = ["the drawing", "arcs, circles and dash patterns", "curves, a fill with a hole and a hatched fill"][which as usize];
                self.yes(m <= 1 && n <= (w * h) / 1000, &format!("{what} at ({}, {}) against at the origin: {n} pixels differ, largest {m}", at[0], at[1]));
            }
            let text = t.deep.text();
            self.yes(t.deep.fine(), &text);
            let text = t.deep_arcs.arc_text();
            self.yes(t.deep_arcs.fine_arcs(), &text);
            for (k, name) in ["moved there", "scaled by 1e5, turned, mirrored"].into_iter().enumerate() {
                let text = format!("an insert {name}: {}", t.deep_ins[k].text());
                self.yes(t.deep_ins[k].fine(), &text);
            }
            let d = &t.deep_curves;
            self.yes(d.checks > 100 && d.lost == 0 && d.worst <= 1.0 / 16.0, &format!("curves at 1e-6 units per pixel near 1e7 (an ellipse of radii 1e7 and 6e6, a Bézier with controls 1e6 away), 24 pans: {} crossings, \
                                                                                  the largest distance from where the curve belongs {:.4} px, {} without it", d.checks, d.worst, d.lost));
            let (l, e) = (&t.hatch_lines, &t.hatch_edge);
            self.yes(l.checks > 100 && l.lost == 0 && l.worst <= 1.0 / 16.0 && e.checks > 100 && e.worst <= 1.0 / 16.0,
                     &format!("a hatch at 1e-6 units per pixel near 1e7, 24 pans: the lines' centroids at most {:.4} px from where they belong, the outline's edge at most {:.4} px", l.worst, e.worst));
        }
        if !t.phase_done {
            t.phase_done = true;
            for &(worst, checks, name) in &t.phase {
                self.yes(checks >= 100 && worst <= 1.0 / 16.0, &format!("dashes at 1e-6 units per pixel, {name}: {checks} pixel lines over 24 pans, the largest distance of a dash's end \
                                                                       from where it belongs {worst:.4} px (1/16 = 0.0625)"));
            }
        }
        while let Some(v) = t.vis.first() {
            let mut lists = alloc::vec![[0u32; 4]; v.layers];
            let mut vis = alloc::vec![0u32; (v.s.visible_buffer().size() / 4) as usize];
            match if v.layers > 0 { v.lists.read(0, &mut lists) } else { Ok(()) } {
                Err(GpuError::NotReady) => return Ok(false),
                r => r?,
            }
            match v.vis.read(0, &mut vis) {
                Err(GpuError::NotReady) => return Ok(false),
                r => r?,
            }
            let imgs = match &v.pair {
                Some((a, b)) => match (a.read(), b.read()) {
                    (Ok(a), Ok(b)) => Some((a, b)),
                    (Err(GpuError::NotReady), _) | (_, Err(GpuError::NotReady)) => return Ok(false),
                    (Err(e), _) | (_, Err(e)) => return Err(e),
                },
                None => None,
            };
            let res = vis_judge(&v.s, &lists, &vis, &v.ents, &v.cam, VIS_SIZE);
            let what = match res {
                Some(res) => (res.fine(), format!("seed {}: {} polylines over {}, rotation {:.2}: {}", v.seed, v.n, v.spread, v.cam.rotation, res.text())),
                None => (false, format!("seed {}: an entity's records", v.seed)),
            };
            self.yes(what.0, &what.1);
            if let Some((a, b)) = imgs {
                let (n, _) = differ(&a, &b);
                self.yes(n == 0, &format!("culled against not culled: {n} pixels differ"));
            }
            t.vis.remove(0);
        }
        Ok(true)
    }

    fn start_bench(&mut self, dev: &Device) -> Result<()> {
        let n: u32 = plat::env("jfgui_GPU_PROBE_N").and_then(|v| v.parse().ok()).unwrap_or(1_000_000);
        let styled = plat::env("jfgui_GPU_PROBE_THIN").is_none();
        let r = Renderer::new(dev, &RendererDesc::new(Format::Bgra8Unorm))?;
        let mut s = Scene::new(&r)?;
        let t0 = plat::time_us();
        let mode = if plat::env("jfgui_GPU_PROBE_PLAIN").is_some() {
            cad::Mode::Plain
        } else if plat::env("jfgui_GPU_PROBE_ARCS").is_some() {
            cad::Mode::Arcs
        } else if plat::env("jfgui_GPU_PROBE_FILLS").is_some() {
            cad::Mode::Fills
        } else {
            cad::Mode::Blocks
        };
        // (the estate: a block of about n records inserted jfgui_GPU_PROBE_INSERTS times, G6b2)
        let c = match plat::env("jfgui_GPU_PROBE_ESTATE") {
            Some(_) => cad::estate(&mut s, plat::env("jfgui_GPU_PROBE_INSERTS").and_then(|v| v.parse().ok()).unwrap_or(10_000), styled)?,
            None => cad::build(&mut s, n, 5, styled, mode)?,
        };
        say(&format!("bench: {} records in {} entities ({} arcs and circles of {} pieces, {} dashed of {} records, {} fills ({} hatched) of {} records, {} curves of {} records, {} blocks inserted {} times: {} records),                       {:.0} x {:.0} m, inserted in {:.0} ms (wasm)",
                     c.segments, c.entities, c.arcs, c.arc_records, c.dashed, c.dashed_records, c.fills, c.hatched, c.fill_records, c.curves, c.curve_records, c.blocks, c.inserts, c.insert_records, c.width, c.height,
                     (plat::time_us() - t0) as f64 / 1000.0));
        let size = (1920, 1080);
        let off = dev.create_texture(&TextureDesc::d2(size.0, size.1, Format::Bgra8Unorm, TextureUsage::COLOR_TARGET))?;
        self.mode = Mode::Bench(Box::new(Bench { r, s, off, used: false, size, cads: [c], styled, n, view: 0, sent: 0, done: 0, flight: Vec::new(), t0: 0, cam: Camera::new([0.0, 0.0], 1.0), cpu_ms: Vec::new() }));
        Ok(())
    }

    fn poll_bench(&mut self, b: &mut Bench, dev: &Device) -> Result<bool> {
        // the frames that finished
        while let Some(&(id, k)) = b.flight.first() {
            if !dev.is_done(id) {
                break;
            }
            b.flight.remove(0);
            b.done += 1;
            if k == WARM - 1 {
                b.t0 = plat::time_us();
            }
            if k == WARM + BENCH_FRAMES - 1 {
                let ms = (plat::time_us() - b.t0) as f64 / 1000.0 / BENCH_FRAMES as f64;
                let cpu = median(&mut b.cpu_ms);
                let v = &cad::views(&b.cads[0], b.size)[b.view];
                say(&format!("bench {} {}: {ms:.2} ms per frame ({:.1} frames/s), {cpu:.2} ms of it wasm (prepare, record, submit); {} segments, {}", v.name, if b.styled { "styled" } else { "hairlines" }, 1000.0 / ms, b.n,
                             if b.view == 0 { "all in view" } else { "partly in view" }));
                b.view += 1;
                b.sent = 0;
                b.done = 0;
                b.cpu_ms.clear();
                if b.view == 3 {
                    return Ok(true);
                }
            }
        }
        // the next frames: up to AHEAD in flight, so that the polling's delay does not count
        while b.flight.len() < AHEAD && b.sent < WARM + BENCH_FRAMES {
            let k = b.sent;
            let view = cad::views(&b.cads[0], b.size);
            let mut cam = view[b.view].cam;
            // (a frame of a pan)
            cam.center[0] += k as f64 * 0.37 * cam.scale;
            b.cam = cam;
            let t = plat::time_us();
            b.s.set_camera(&cam)?;
            b.s.prepare(&mut b.r, b.size)?;
            let mut e = dev.encoder()?;
            let color = ColorAttachment { view: b.off.view(), load: Load::Clear, clear: bg_clear(BG), store: true, before: if b.used { Layout::ColorTarget } else { Layout::Undefined }, after: Layout::ColorTarget };
            b.r.record(&mut e, &b.s, color)?;
            let id = dev.queue().submit([e.finish()?])?;
            b.s.submitted(id);
            b.used = true;
            b.flight.push((id, k));
            b.sent += 1;
            b.cpu_ms.push((plat::time_us() - t) as f64 / 1000.0);
        }
        Ok(false)
    }

    fn step(&mut self) -> Result<()> {
        match self.step {
            0 => {
                match Instance::new() {
                    Ok(i) => self.inst = Some(i),
                    Err(e) => {
                        say(&format!("SKIP render2dtest: {e}"));
                        self.step = u32::MAX;
                        return Ok(());
                    }
                }
                self.step = 1;
            }
            1 => {
                let Some(inst) = self.inst.clone() else { return Ok(()) };
                match inst.prepare() {
                    Ok(false) => return Ok(()),
                    Ok(true) => {}
                    Err(e) => {
                        say(&format!("SKIP render2dtest: {e}"));
                        self.step = u32::MAX;
                        return Ok(());
                    }
                }
                let a = inst.adapter()?;
                let dev = a.create_device(&DeviceDesc::DEFAULT)?;
                let (i, l) = (a.info(), dev.limits());
                say(&format!("adapter: {} ({}), level {:?}, storage binding {} MiB", i.name, i.backend, dev.level(), l.max_storage_binding >> 20));
                if self.probe == "bench" {
                    self.limit_s = 600;
                    self.start_bench(&dev)?;
                } else {
                    self.start_test(&dev)?;
                }
                self.dev = Some(dev);
                self.step = 2;
            }
            2 => {
                let Some(dev) = self.dev.clone() else { return Ok(()) };
                dev.poll()?;
                let finished = match core::mem::replace(&mut self.mode, Mode::None) {
                    Mode::Test(mut t) => {
                        let f = self.poll_test(&mut t)?;
                        self.mode = Mode::Test(t);
                        f
                    }
                    Mode::Bench(mut b) => {
                        let f = self.poll_bench(&mut b, &dev)?;
                        self.mode = Mode::Bench(b);
                        f
                    }
                    Mode::None => true,
                };
                if finished {
                    dev.poll()?;
                    self.yes(dev.poll().is_ok(), "the browser found no validation error in all of it");
                    self.end();
                }
            }
            _ => {}
        }
        Ok(())
    }
}

fn median(v: &mut [f64]) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(core::cmp::Ordering::Equal));
    v[v.len() / 2]
}

impl plat::web::WebApplication for Probe {
    fn frame_delay(&self) -> u32 {
        0
    }
    fn tick(&mut self, _time_ms: u32) {
        if self.step == u32::MAX {
            return;
        }
        if plat::time_us() > self.t0 + self.limit_s * 1_000_000 {
            self.yes(false, "the browser's answers did not come in time");
            self.end();
            return;
        }
        if let Err(e) = self.step() {
            let what = format!("step {}: {e}", self.step);
            self.yes(false, &what);
            self.end();
        }
    }
    fn control(&mut self, _line: &[u8]) -> Option<String> {
        None
    }
}

/// the probe instead of the demo when the address asks for it
pub fn wanted() -> Option<String> {
    plat::env("jfgui_GPU_PROBE")
}

pub fn start(probe: String) -> i32 {
    let t0 = plat::time_us();
    let p = Probe { step: 0, probe, inst: None, dev: None, mode: Mode::None, ok: 0, bad: 0, t0, limit_s: 120 };
    if !plat::web::install(Box::new(p)) {
        return 2;
    }
    0
}
