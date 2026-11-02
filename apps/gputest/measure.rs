// measure -- gputest --bench N: the frame loop of the benchmark scene (scene.rs) and its
// statistics (docs/GPU.md "Messszene")
//
//   gputest --bench N [--res 1080p|4k|WxH] [--mode indirect|single] [--window | --fullscreen]
//                     [--present immediate|fifo|mailbox] [--frames F] [--warmup W]
//                     [--inflight K] [--verify] [--shot FILE.png] [--validate]
//
// Defaults: 1080p, indirect, offscreen, 300 measured frames after 60 warm-up frames, 3 frames
// in flight, IMMEDIATE in a window.  Offscreen: an Rgba8Unorm colour target and a Depth32Float
// target of the size, no presentation.  --window: a plat window with that client size (at most
// the monitor's work area) and a swapchain; --fullscreen: a window over the whole monitor
// without a frame (its size is the monitor's).  The animation time advances 1/60 s per frame
// (not the wall clock), so every run draws the same frames.
//
// Frames in flight: the scene has K frame slots (camera staging, DrawCmd read-back,
// timestamps).  Before recording frame f into slot f % K the loop waits for the submission of
// frame f - K (offscreen: device.wait; window: swapchain.acquire, created with
// frames_in_flight K), reads that frame's timestamps and instance count, then records and
// submits.  So the CPU runs up to K frames ahead and waits only when the GPU is behind.
// Per frame: rec = writing the camera + recording + finish; cpu = rec + submit (window:
// + present), no waits; wait = the time spent waiting for the slot; frame = the interval
// between two submits (presents); GPU: move, cull, draw and total (timestamp 3 - timestamp 0)
// from the timestamps, gpu frame = the interval between two frames' last timestamps, busy =
// the sum of the totals over the sum of those intervals.  Median, 99th percentile (and the
// frame's mean) over the measured frames.  --verify: after the loop, the objects and the last frame's
// visible list against move_all / cull_ref on the CPU, bit for bit.

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use gpu::*;
use shader::bench;

use crate::scene::{Mode, Scene, Shaders, bench_camera};
use crate::{Gpu, T, out};

#[derive(Clone, Debug)]
pub struct Config {
    pub n: u32,
    pub w: u32,
    pub h: u32,
    pub mode: Mode,
    /// Some(mode): a window with a swapchain
    pub window: Option<PresentMode>,
    pub frames: usize,
    pub warmup: usize,
    pub inflight: usize,
    pub verify: bool,
    /// offscreen: the last frame as a PNG file
    pub shot: Option<String>,
    /// the window covers its monitor without a frame (plat WCMD_FULLSCREEN; the size is the
    /// monitor's)
    pub fullscreen: bool,
    /// --shaders spirv | wgsl, --robust 0 (WGSL without index clamping), --fused (WGSL move and
    /// cull without NoContraction)
    pub shaders: Shaders,
}

impl Config {
    pub fn new(n: u32) -> Config {
        Config { n, w: 1920, h: 1080, mode: Mode::Indirect, window: None, frames: 300, warmup: 60, inflight: 3, verify: false,
                 shot: None, fullscreen: false, shaders: Shaders::Spirv }
    }
}

/// median, 99th percentile and mean
#[derive(Clone, Copy, Debug, Default)]
pub struct Stat {
    pub med: f64,
    pub p99: f64,
    pub mean: f64,
}

pub fn stat(v: &[f64]) -> Stat {
    if v.is_empty() {
        return Stat::default();
    }
    let mut s = v.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap_or(core::cmp::Ordering::Equal));
    Stat { med: s[s.len() / 2], p99: s[(s.len() * 99 / 100).min(s.len() - 1)], mean: s.iter().sum::<f64>() / s.len() as f64 }
}

/// what a run measured
#[derive(Clone, Debug, Default)]
pub struct Report {
    pub w: u32,
    pub h: u32,
    pub frames: usize,
    pub visible: Stat,
    /// writing the camera, recording and finish (no submit)
    pub rec: Stat,
    pub cpu: Stat,
    pub wait: Stat,
    pub frame: Stat,
    pub gpu_move: Stat,
    pub gpu_cull: Stat,
    pub gpu_draw: Stat,
    pub gpu_total: Stat,
    pub gpu_frame: Stat,
    /// the GPU's busy share: the sum of the frames' GPU totals over the sum of their intervals
    pub busy: f64,
    /// "offscreen" or the window's present mode
    pub out: String,
    pub timestamps: bool,
}

#[derive(Clone, Copy, Default)]
struct Rec {
    rec: f64,
    cpu: f64,
    wait: f64,
    interval: f64,
    ts: Option<[f64; 4]>,
    visible: u32,
}

/// parse the options after --bench N
pub fn parse(n: u32) -> core::result::Result<Config, String> {
    let mut c = Config::new(n);
    if let Some(r) = plat::arg_value("--res") {
        (c.w, c.h) = match r.as_str() {
            "1080p" => (1920, 1080),
            "4k" | "2160p" => (3840, 2160),
            "720p" => (1280, 720),
            s => {
                let mut p = s.split('x').map(|x| x.parse::<u32>().ok());
                match (p.next().flatten(), p.next().flatten()) {
                    (Some(w), Some(h)) if w > 0 && h > 0 && w <= 16384 && h <= 16384 => (w, h),
                    _ => return Err(format!("--res {s}: 1080p, 4k, 720p or WxH")),
                }
            }
        };
    }
    if let Some(m) = plat::arg_value("--mode") {
        c.mode = match m.as_str() {
            "indirect" => Mode::Indirect,
            "single" => Mode::Single,
            _ => return Err(format!("--mode {m}: indirect or single")),
        };
    }
    let args = plat::args();
    if args.iter().any(|a| a == "--window") {
        c.window = Some(PresentMode::Immediate);
    }
    if let Some(p) = plat::arg_value("--present") {
        c.window = Some(match p.as_str() {
            "immediate" => PresentMode::Immediate,
            "fifo" => PresentMode::Fifo,
            "mailbox" => PresentMode::Mailbox,
            _ => return Err(format!("--present {p}: immediate, fifo or mailbox")),
        });
    }
    let num = |name: &str, lo: usize, hi: usize| -> core::result::Result<Option<usize>, String> {
        match plat::arg_value(name) {
            None => Ok(None),
            Some(v) => match v.parse::<usize>() {
                Ok(x) if (lo..=hi).contains(&x) => Ok(Some(x)),
                _ => Err(format!("{name} {v}: {lo} ..= {hi}")),
            },
        }
    };
    c.frames = num("--frames", 1, 1_000_000)?.unwrap_or(c.frames);
    c.warmup = num("--warmup", 0, 100_000)?.unwrap_or(c.warmup);
    c.inflight = num("--inflight", 1, 3)?.unwrap_or(c.inflight);
    c.verify = args.iter().any(|a| a == "--verify");
    c.shot = plat::arg_value("--shot");
    let robust = plat::arg_value("--robust").is_none_or(|v| v != "0");
    let exact = !args.iter().any(|a| a == "--fused");
    if let Some(s) = plat::arg_value("--shaders") {
        c.shaders = match s.as_str() {
            "spirv" => Shaders::Spirv,
            "wgsl" => Shaders::Wgsl { robust, exact },
            _ => return Err(format!("--shaders {s}: spirv or wgsl")),
        };
    }
    if args.iter().any(|a| a == "--fullscreen") {
        c.fullscreen = true;
        c.window = c.window.or(Some(PresentMode::Immediate));
    }
    Ok(c)
}

const CLEAR: [f32; 4] = [0.05, 0.06, 0.09, 1.0];

/// the result of frame f from its slot (its submission has finished)
fn collect(scene: &Scene, slot: usize, r: &mut Rec, mode: Mode) {
    r.ts = scene.gpu_ms(slot);
    r.visible = if mode == Mode::Indirect { scene.visible(slot) } else { scene.n };
}

/// handle the window's messages -> false when it was closed or Esc pressed
fn pump_events() -> bool {
    plat::wait(0);
    let mut go = true;
    while let Some(ev) = plat::evq_pop() {
        if ev.typ == plat::E_CLOSE || (ev.typ == plat::E_KEY_DOWN && ev.data & 0x7FFF_FFFF == 0x1B) {
            go = false;
        }
    }
    go
}

fn time_of(f: usize) -> f32 {
    1.0 + f as f32 / 60.0
}

/// run the scene with c -> the report (None: a failure, recorded in t)
pub fn run(t: &mut T, g: &Gpu, c: &Config) -> Option<Report> {
    let dev = &g.dev;
    let k = c.inflight.clamp(1, 3);
    let total = c.warmup + c.frames;
    let mut recs: Vec<Rec> = vec![Rec::default(); total];
    let mut ids: Vec<Option<(SubmitId, usize)>> = vec![None; k];
    let mut rep = Report { frames: c.frames, timestamps: dev.features().timestamps, ..Report::default() };
    let mut last_t = 0.0f32;
    let scene;
    if c.window.is_some() && !dev.features().swapchain {
        t.fail("--window: no swapchains on this device");
        return None;
    }
    if let Some(present) = c.window {
        // a plat window of the size (within the work area) and a swapchain
        let w = plat::win_create("gputest --bench", 640, 360, plat::PWF_SYSFRAME, plat::PW_NOOWNER, plat::PW_DEFAULT,
                                 plat::PW_DEFAULT);
        if !t.ok(w >= 0, "plat window") {
            return None;
        }
        let (mut cw, mut ch) = (c.w as i32, c.h as i32);
        if let Some(a) = plat::win_work_area(w) {
            cw = cw.min(a[2] - 16);
            ch = ch.min(a[3] - 48);
            plat::win_set_rect(w, a[0] + 8, a[1] + 40, cw, ch);
        } else {
            plat::win_set_rect(w, plat::PW_KEEP, plat::PW_KEEP, cw, ch);
        }
        plat::win_show(w, plat::PWS_SHOW);
        if c.fullscreen {
            plat::win_cmd(w, plat::WCMD_FULLSCREEN);
        }
        crate::t_window::pump(200);
        let r = (|| {
            let surf = t.res(Surface::new(&g.inst, w), "Surface::new")?;
            // (Linux: no display in this session, or it runs on another GPU)
            if cfg!(not(windows)) && !g.adapter.can_present(&surf) {
                out("SKIP: this adapter cannot show on the window's display");
                return None;
            }
            let desc = SwapchainDesc { present, frames_in_flight: k as u32, ..SwapchainDesc::DEFAULT };
            let mut sc = t.res(dev.create_swapchain(&surf, &desc), "create_swapchain")?;
            let s = Scene::new(t, dev, c.n, sc.format(), k, c.shaders)?;
            rep.out = format!("{:?}", sc.present_mode()).to_lowercase();
            let mut depth: Option<Texture> = None;
            let mut last = plat::time_us();
            let mut f = 0;
            while f < total {
                if !pump_events() {
                    out("  window closed: stopped");
                    break;
                }
                let tw = plat::time_us();
                let Some(fr) = t.res(sc.acquire(), "acquire")? else {
                    crate::t_window::pump(10);
                    continue;
                };
                let slot = f % k;
                if let Some((id, pf)) = ids[slot] {
                    t.res(dev.wait(id, 10_000_000_000), "wait")?; // done already: acquire waited for it
                    collect(&s, slot, &mut recs[pf], c.mode);
                }
                let t0 = plat::time_us();
                let size = fr.size();
                let fresh = depth.as_ref().is_none_or(|d| (d.width(), d.height()) != size);
                if fresh {
                    depth = Some(t.res(dev.create_texture(&TextureDesc::d2(size.0, size.1, Format::Depth32Float,
                                                                           TextureUsage::DEPTH_TARGET)), "depth target")?);
                }
                let dt = depth.as_ref()?;
                let time = time_of(f);
                last_t = time;
                t.res(s.set_camera(slot, &bench_camera(time, s.extent, size.0 as f32 / size.1 as f32)), "camera")?;
                let mut e = t.res(dev.encoder(), "encoder")?;
                let color = ColorAttachment { view: sc.view(&fr), load: Load::Clear, clear: CLEAR, store: true,
                                              before: Layout::Undefined, after: Layout::Present };
                let da = DepthAttachment { view: dt.view(), load: Load::Clear, clear: 1.0, store: false,
                                           before: if fresh { Layout::Undefined } else { Layout::DepthTarget },
                                           after: Layout::DepthTarget };
                s.record(&mut e, slot, c.mode, Some(time), color, da);
                let cmd = t.res(e.finish(), "finish")?;
                let tr = plat::time_us();
                let id = t.res(sc.present(fr, [cmd]), "present")?;
                let t1 = plat::time_us();
                recs[f] = Rec { rec: (tr - t0) as f64 / 1000.0, cpu: (t1 - t0) as f64 / 1000.0, wait: (t0 - tw) as f64 / 1000.0,
                                interval: (t1 - last) as f64 / 1000.0, ..Rec::default() };
                last = t1;
                ids[slot] = Some((id, f));
                f += 1;
                rep.w = size.0;
                rep.h = size.1;
            }
            t.res(dev.wait_idle(), "wait_idle")?;
            for slot in 0..k {
                if let Some((_, pf)) = ids[slot] {
                    collect(&s, slot, &mut recs[pf], c.mode);
                }
            }
            recs.truncate(f);
            drop(sc);
            Some(s)
        })();
        plat::win_destroy(w);
        crate::t_window::pump(20);
        scene = r?;
    } else {
        let color = t.res(dev.create_texture(&TextureDesc::d2(c.w, c.h, Format::Rgba8Unorm,
                                                              TextureUsage::COLOR_TARGET | TextureUsage::COPY_SRC)), "colour target")?;
        let depth = t.res(dev.create_texture(&TextureDesc::d2(c.w, c.h, Format::Depth32Float, TextureUsage::DEPTH_TARGET)),
                          "depth target")?;
        let s = Scene::new(t, dev, c.n, Format::Rgba8Unorm, k, c.shaders)?;
        rep.out = String::from("offscreen");
        rep.w = c.w;
        rep.h = c.h;
        let aspect = c.w as f32 / c.h as f32;
        let mut last = plat::time_us();
        for f in 0..total {
            let slot = f % k;
            let tw = plat::time_us();
            if let Some((id, pf)) = ids[slot] {
                t.res(dev.wait(id, 10_000_000_000), "wait")?;
                collect(&s, slot, &mut recs[pf], c.mode);
            }
            let t0 = plat::time_us();
            let time = time_of(f);
            last_t = time;
            t.res(s.set_camera(slot, &bench_camera(time, s.extent, aspect)), "camera")?;
            let mut e = t.res(dev.encoder(), "encoder")?;
            let first = f == 0;
            let ca = ColorAttachment { view: color.view(), load: Load::Clear, clear: CLEAR, store: true,
                                       before: if first { Layout::Undefined } else { Layout::ColorTarget },
                                       after: Layout::ColorTarget };
            let da = DepthAttachment { view: depth.view(), load: Load::Clear, clear: 1.0, store: false,
                                       before: if first { Layout::Undefined } else { Layout::DepthTarget },
                                       after: Layout::DepthTarget };
            s.record(&mut e, slot, c.mode, Some(time), ca, da);
            let cmd = t.res(e.finish(), "finish")?;
            let tr = plat::time_us();
            let id = t.res(dev.queue().submit([cmd]), "submit")?;
            let t1 = plat::time_us();
            recs[f] = Rec { rec: (tr - t0) as f64 / 1000.0, cpu: (t1 - t0) as f64 / 1000.0, wait: (t0 - tw) as f64 / 1000.0,
                            interval: (t1 - last) as f64 / 1000.0, ..Rec::default() };
            last = t1;
            ids[slot] = Some((id, f));
        }
        t.res(dev.wait_idle(), "wait_idle")?;
        for slot in 0..k {
            if let Some((_, pf)) = ids[slot] {
                collect(&s, slot, &mut recs[pf], c.mode);
            }
        }
        if let Some(path) = &c.shot {
            shot(t, dev, &color, path);
        }
        drop(color);
        drop(depth);
        scene = s;
    }
    // statistics over the measured frames
    let m = &recs[c.warmup.min(recs.len())..];
    let pick = |f: &dyn Fn(&Rec) -> Option<f64>| -> Vec<f64> { m.iter().filter_map(f).collect() };
    rep.frames = m.len();
    rep.rec = stat(&pick(&|r| Some(r.rec)));
    rep.cpu = stat(&pick(&|r| Some(r.cpu)));
    rep.wait = stat(&pick(&|r| Some(r.wait)));
    rep.frame = stat(&pick(&|r| Some(r.interval)));
    rep.visible = stat(&pick(&|r| Some(r.visible as f64)));
    rep.gpu_move = stat(&pick(&|r| r.ts.map(|s| s[1] - s[0])));
    rep.gpu_cull = stat(&pick(&|r| r.ts.map(|s| s[2] - s[1])));
    rep.gpu_draw = stat(&pick(&|r| r.ts.map(|s| s[3] - s[2])));
    rep.gpu_total = stat(&pick(&|r| r.ts.map(|s| s[3] - s[0])));
    let start = c.warmup.min(recs.len());
    let gf: Vec<f64> = (start.max(1)..recs.len())
        .filter_map(|f| Some(recs[f].ts?[3] - recs[f - 1].ts?[3]))
        .collect();
    rep.gpu_frame = stat(&gf);
    let busy: f64 = (start.max(1)..recs.len()).filter_map(|f| Some(recs[f].ts?[3] - recs[f].ts?[0])).sum();
    rep.busy = if gf.is_empty() { 0.0 } else { busy / gf.iter().sum::<f64>() };
    if c.verify && !recs.is_empty() {
        verify(t, dev, &scene, last_t, c, rep.w, rep.h);
    }
    Some(rep)
}

/// the objects after the last frame (time) and its visible list against the CPU references
fn verify(t: &mut T, dev: &Device, s: &Scene, time: f32, c: &Config, w: u32, h: u32) {
    let n = s.n as usize;
    let mut got = vec![0u32; n * 12];
    if t.res(dev.queue().read_buffer(&s.objects, 0, &mut got), "read objects").is_none() {
        return;
    }
    let objs = bench::move_all(&s.push(time));
    let want: Vec<u32> = objs.iter().flat_map(|o| o.bytes()).collect::<Vec<u8>>().chunks_exact(4)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect();
    let (exact, flushed, bad, first) = compare_words(&got, &want);
    t.ok(bad == 0, &format!("--verify: {n} objects after the last frame (t = {time}) = move_all: {exact} floats equal, \
                             {flushed} denormals flushed, {bad} different{}",
                            first.map_or(String::new(), |i| format!(" (first: object {}, word {})", i / 12, i % 12))));
    if c.mode != Mode::Indirect {
        return;
    }
    let cam = bench_camera(time, s.extent, w as f32 / h as f32);
    let mut d = [0u32; 5];
    if t.res(dev.queue().read_buffer(&s.draw, 0, &mut d), "read draw command").is_none() {
        return;
    }
    let want = bench::cull_ref(&objs, &cam);
    let mut list = vec![0u32; d[1] as usize];
    if !list.is_empty() && t.res(dev.queue().read_buffer(&s.visible, 0, &mut list), "read visible").is_none() {
        return;
    }
    list.sort_unstable();
    t.ok(d[1] as usize == want.len() && list == want,
         &format!("--verify: the last frame's visible list = cull_ref ({} of {n}; CPU {})", d[1], want.len()));
}

/// the colour target (in Layout::ColorTarget, the GPU idle) as an RGB PNG file
fn shot(t: &mut T, dev: &Device, color: &Texture, path: &str) {
    let (w, h) = (color.width(), color.height());
    let Some(rb) = crate::t_compute::readback(t, dev, (w * h * 4) as u64) else { return };
    let Some(mut e) = t.res(dev.encoder(), "encoder") else { return };
    e.texture_barrier(color, Layout::ColorTarget, Layout::CopySrc);
    e.copy_texture_to_buffer(TextureRegion::mip(color, 0), &rb, DataLayout::default());
    e.barrier(Access::COPY_WRITE, Access::HOST_READ);
    if crate::run_wait(t, dev, e.finish(), "shot").is_none() {
        return;
    }
    let mut px = vec![0u8; (w * h * 4) as usize];
    if t.res(rb.read(0, &mut px), "read shot").is_some() {
        let ok = plat::write_file(path, &png_rgba(&px, w, h));
        t.ok(ok, &format!("--shot: {path} written ({w}x{h})"));
    }
}

/// a PNG file (8-bit RGB, stored deflate blocks) of RGBA rows
pub fn png_rgba(px: &[u8], w: u32, h: u32) -> Vec<u8> {
    let mut crc_t = [0u32; 256];
    for (i, c) in crc_t.iter_mut().enumerate() {
        let mut v = i as u32;
        for _ in 0..8 {
            v = if v & 1 != 0 { 0xEDB8_8320 ^ (v >> 1) } else { v >> 1 };
        }
        *c = v;
    }
    let crc = |d: &[u8]| !d.iter().fold(!0u32, |c, &b| crc_t[((c ^ b as u32) & 255) as usize] ^ (c >> 8));
    let mut raw = Vec::with_capacity((w * 3 + 1) as usize * h as usize);
    for row in px.chunks_exact(w as usize * 4).take(h as usize) {
        raw.push(0);
        for p in row.chunks_exact(4) {
            raw.extend_from_slice(&p[..3]);
        }
    }
    let mut z = vec![0x78, 0x01];
    let mut chunks = raw.chunks(65535).peekable();
    while let Some(b) = chunks.next() {
        let n = b.len() as u16;
        z.push(chunks.peek().is_none() as u8);
        z.extend_from_slice(&n.to_le_bytes());
        z.extend_from_slice(&(!n).to_le_bytes());
        z.extend_from_slice(b);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for &x in &raw {
        a = (a + x as u32) % 65521;
        b = (b + a) % 65521;
    }
    z.extend_from_slice(&((b << 16) | a).to_be_bytes());
    let mut o = vec![0x89, b'P', b'N', b'G', 13, 10, 26, 10];
    let chunk = |o: &mut Vec<u8>, ty: &[u8; 4], d: &[u8]| {
        o.extend_from_slice(&(d.len() as u32).to_be_bytes());
        let mut c = ty.to_vec();
        c.extend_from_slice(d);
        o.extend_from_slice(&c);
        o.extend_from_slice(&crc(&c).to_be_bytes());
    };
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&w.to_be_bytes());
    ihdr.extend_from_slice(&h.to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
    chunk(&mut o, b"IHDR", &ihdr);
    chunk(&mut o, b"IDAT", &z);
    chunk(&mut o, b"IEND", &[]);
    o
}

/// float words GPU vs CPU -> (equal, denormals flushed to zero, different, first difference)
pub fn compare_words(got: &[u32], want: &[u32]) -> (usize, usize, usize, Option<usize>) {
    let (mut eq, mut fl, mut bad, mut first) = (0, 0, 0, None);
    for (i, (&g, &w)) in got.iter().zip(want).enumerate() {
        if g == w {
            eq += 1;
        } else if w & 0x7F80_0000 == 0 && g & 0x7FFF_FFFF == 0 {
            fl += 1;
        } else {
            bad += 1;
            first.get_or_insert(i);
        }
    }
    let missing = want.len().saturating_sub(got.len());
    if missing > 0 {
        bad += missing;
        first.get_or_insert(got.len());
    }
    (eq, fl, bad, first)
}

pub fn print(c: &Config, r: &Report) {
    let mode = if c.mode == Mode::Indirect { "indirect" } else { "single" };
    let vis = r.visible.med / c.n.max(1) as f64;
    out(&format!("scene: {} cubes, {}x{} {}, {mode}, {} shaders, {} frames in flight, {} frames after {} warm-up", c.n, r.w, r.h,
                 r.out, c.shaders.name(), c.inflight, r.frames, c.warmup));
    out(&format!("  visible: {:.0} cubes (median) = {:.1} %", r.visible.med, vis * 100.0));
    out(&format!("  CPU: record {:.3} ms, record + submit {:.3} ms median, {:.3} p99; waiting for the GPU: {:.3} ms median",
                 r.rec.med, r.cpu.med, r.cpu.p99, r.wait.med));
    if r.timestamps {
        out(&format!("  GPU: move {:.3} / cull {:.3} / draw {:.3} / total {:.3} ms median; p99 {:.3} / {:.3} / {:.3} / {:.3}",
                     r.gpu_move.med, r.gpu_cull.med, r.gpu_draw.med, r.gpu_total.med, r.gpu_move.p99, r.gpu_cull.p99,
                     r.gpu_draw.p99, r.gpu_total.p99));
        out(&format!("  GPU frame interval {:.3} ms median, {:.3} p99; GPU busy {:.1} %", r.gpu_frame.med, r.gpu_frame.p99,
                     r.busy * 100.0));
    }
    let fps = if r.frame.mean > 0.0 { 1000.0 / r.frame.mean } else { 0.0 };
    let tris = r.visible.med * 12.0 * fps;
    out(&format!("  frame: {:.3} ms mean ({fps:.0} fps), {:.3} median, {:.3} p99; {:.2} G triangles/s submitted", r.frame.mean,
                 r.frame.med, r.frame.p99, tris / 1e9));
    out(&format!(
        "RESULT n={} res={}x{} mode={mode} shaders={} out={} inflight={} frames={} vis={:.4} rec={:.4}/{:.4} cpu={:.4}/{:.4} \
         wait={:.4}/{:.4} move={:.4}/{:.4} cull={:.4}/{:.4} draw={:.4}/{:.4} gpu={:.4}/{:.4} gpuframe={:.4}/{:.4} \
         frame={:.4}/{:.4} framemean={:.4} busy={:.3} gtris={:.3}",
        c.n, r.w, r.h, c.shaders.name().replace(' ', "_"), r.out, c.inflight, r.frames, vis, r.rec.med, r.rec.p99, r.cpu.med, r.cpu.p99, r.wait.med, r.wait.p99,
        r.gpu_move.med, r.gpu_move.p99, r.gpu_cull.med, r.gpu_cull.p99, r.gpu_draw.med, r.gpu_draw.p99, r.gpu_total.med,
        r.gpu_total.p99, r.gpu_frame.med, r.gpu_frame.p99, r.frame.med, r.frame.p99, r.frame.mean, r.busy, tris / 1e9
    ));
}

/// gputest --bench N ...: one run -> exit code
pub fn bench_scene(n: u32, validate: bool) -> i32 {
    let c = match parse(n) {
        Ok(c) => c,
        Err(e) => {
            plat::write_stderr(format!("gputest: {e}\n").as_bytes());
            return 2;
        }
    };
    let mut t = T { fails: 0, validate, expected: 0 };
    let Some(g) = crate::setup(&mut t) else { return if t.fails > 0 { 1 } else { 0 } };
    out(&format!("{} ({}), level {:?}", g.dev.info().name, g.dev.info().driver_info, g.dev.level()));
    if let Some(r) = run(&mut t, &g, &c) {
        print(&c, &r);
    }
    if validate {
        let (e, w) = gpu::validation_messages();
        t.ok(e == 0, &format!("validation layer: {e} errors, {w} warnings"));
    }
    if t.fails > 0 { 1 } else { 0 }
}
