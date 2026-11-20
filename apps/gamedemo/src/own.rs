// own -- (G7b part 2) path (b) of the game window: a window and a swapchain of its own, no lib/ui; the HUD a
// texture over the 3D picture (docs/GAMECORE.md "Spielfenster")
//
//   gamedemo --bench own [--present fifo|mailbox|immediate|all] [--fullscreen] [--limit HZ] [--n N] [--secs S]
//            [--steps none,hud]
//
// A plat window of 1280 x 800 (shown without activation), a device at level Voll on the fastest adapter that can
// show it (as lib/ui's AF_GPU_3D), a swapchain with two frames in flight.  The loop: the window's events
// (game::Input::plat_event), a frame limiter (--limit: the loop waits until the frame is due, then takes the input),
// the ticks, render3d straight into the swapchain's image, the HUD over it (one pass, premultiplied alpha from a
// texture the CPU fills with gfx and uploads when it changes), present.  The present modes one after the other in
// the same window (all: FIFO, MAILBOX, IMMEDIATE).  The probe (probe.rs) and the steps as in bench.rs.
//
// What this path does not have: lib/ui.  Its widgets draw into an opaque framebuffer (subpixel text, the CPU's
// blending), so a menu or a HUD of lib/ui as a texture with alpha would need lib/ui to draw into one; the HUD here
// is the game's own, drawn with gfx (rectangles, no text).

use alloc::format;
use alloc::vec::Vec;

use game::{Pose, Vec3};
use game_render3d::Bridge;
use gpu::*;

use crate::bench::{Hud, Next, Plan, TURN};
use crate::{arg, has, View, EYE};

/// the HUD's shader: its texture one to one over the window, premultiplied
const HUD_WGSL: &str = concat!(
    "@group(0) @binding(0) var hud: texture_2d<f32>;\n",
    "@vertex\n",
    "fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4f {\n",
    "    let x = f32((i << 1u) & 2u);\n",
    "    let y = f32(i & 2u);\n",
    "    return vec4f(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);\n",
    "}\n",
    "@fragment\n",
    "fn fs(@builtin(position) p: vec4f) -> @location(0) vec4f {\n",
    "    return textureLoad(hud, vec2i(p.xy), 0);\n",
    "}\n",
);

/// the HUD on the GPU: its texture (the window's size), the rectangles that hold something
struct HudGpu {
    pipe: RenderPipeline,
    layout: BindGroupLayout,
    tex: Option<(Texture, BindGroup, u32, u32)>,
    sf: gfx::Surface,
    rects: Vec<[i32; 4]>,
    /// what it shows now (health): drawn again when it changes
    shown: i32,
}

fn shaders(dev: &Device) -> Result<(ShaderModule, ShaderModule)> {
    #[cfg(target_arch = "wasm32")]
    {
        let m = ShaderModule::from_wgsl(dev, HUD_WGSL)?;
        Ok((m.clone(), m))
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let o = wgsl::compile(HUD_WGSL, &wgsl::CompileOptions::new(shader::BASIS)).map_err(|_| GpuError::Invalid("gamedemo: the HUD's WGSL"))?;
        let words = |n: &str| o.entry(n).map(|e| e.words.clone()).ok_or(GpuError::Invalid("gamedemo: no entry point"));
        Ok((ShaderModule::from_spirv(dev, &words("vs")?)?, ShaderModule::from_spirv(dev, &words("fs")?)?))
    }
}

impl HudGpu {
    fn new(dev: &Device, format: Format) -> Result<HudGpu> {
        let layout = dev.create_bind_group_layout(&[Binding { binding: 0, kind: BindingKind::Texture, stages: Stages::FRAGMENT }])?;
        let pl = dev.create_pipeline_layout(&PipelineLayoutDesc { groups: &[&layout], push_constants: 0 })?;
        let (vs, fs) = shaders(dev)?;
        let pipe = dev.create_render_pipeline(&RenderPipelineDesc {
            layout: &pl,
            vertex: ShaderStage::new(&vs, "vs"),
            fragment: Some(ShaderStage::new(&fs, "fs")),
            buffers: &[],
            topology: Topology::Triangles,
            cull: Cull::None,
            front_cw: false,
            targets: &[ColorTarget { format, blend: Some(Blend::PREMULTIPLIED), write_mask: 0xF }],
            depth: None,
        })?;
        Ok(HudGpu { pipe, layout, tex: None, sf: gfx::Surface::default(), rects: Vec::new(), shown: -1 })
    }

    /// the HUD for a target of w x h showing `health`: drawn and uploaded when either changed
    fn update(&mut self, dev: &Device, (w, h): (u32, u32), health: i32) -> Result<()> {
        let fresh = self.tex.as_ref().is_none_or(|t| (t.2, t.3) != (w, h));
        if !fresh && health == self.shown {
            return Ok(());
        }
        let (wi, hi) = (w as i32, h as i32);
        if fresh {
            let t = dev.create_texture(&TextureDesc::d2(w, h, Format::Bgra8Unorm, TextureUsage::COPY_DST | TextureUsage::SAMPLED))?;
            let g = dev.create_bind_group(&BindGroupDesc { layout: &self.layout, entries: &[BindEntry { binding: 0, resource: Resource::Texture(t.view()) }] })?;
            self.tex = Some((t, g, w, h));
            self.sf = gfx::Surface::new(wi, hi, gfx::FMT_PARGB);
        }
        // the same pieces as the HUD of lib/ui (main.rs hud), without text: a crosshair, a bar, a panel
        let (cx, cy) = (wi / 2, hi / 2);
        let (bx, by) = (24, hi - 44);
        let (pw, ph) = (190, 52);
        let (px, py) = (wi - pw - 20, 20);
        let rects = [[cx - 15, cy - 15, 30, 30], [bx, by, 200, 16], [px, py, pw, ph]];
        let mut dc = gfx::Dc::new(wi, hi);
        let px_clear = |sf: &mut gfx::Surface, r: [i32; 4]| {
            for y in r[1].max(0)..(r[1] + r[3]).min(sf.h) {
                let at = y as usize * sf.stride as usize + r[0].max(0) as usize * 4;
                let n = (r[2].min(sf.w - r[0].max(0))).max(0) as usize * 4;
                sf.px[at..at + n].fill(0);
            }
        };
        for r in rects {
            px_clear(&mut self.sf, r);
        }
        dc.set_color(0xF0FF_FFFF);
        for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            let (x0, y0) = (cx + dx * 5, cy + dy * 5);
            let (x1, y1) = (cx + dx * 14, cy + dy * 14);
            dc.fill(&mut self.sf, x0.min(x1) - (dy != 0) as i32, y0.min(y1) - (dx != 0) as i32, (x1 - x0).abs().max(2), (y1 - y0).abs().max(2));
        }
        dc.set_color(0x8010_1418);
        dc.fill(&mut self.sf, bx, by, 200, 16);
        dc.set_color(0xFF4C_D964);
        dc.fill(&mut self.sf, bx, by, 200 * health.clamp(0, 100) / 100, 16);
        dc.set_color(0xB01A_1E28);
        dc.fill(&mut self.sf, px, py, pw, ph);
        dc.set_color(0xFFE8_ECF0);
        dc.fill(&mut self.sf, px + 12, py + 12, (pw - 24) * health.clamp(0, 100) / 100, 6);
        let up: Vec<[i32; 4]> = rects.iter().map(|r| clip(*r, wi, hi)).filter(|r| r[2] > 0 && r[3] > 0).collect();
        if let Some((t, ..)) = &self.tex {
            let before = if fresh { Layout::Undefined } else { Layout::Sampled };
            if fresh {
                dev.queue().write_texture_rects(t, &[[0, 0, wi, hi]], &self.sf.px, self.sf.stride as u32, before, Layout::Sampled)?;
            } else {
                dev.queue().write_texture_rects(t, &up, &self.sf.px, self.sf.stride as u32, before, Layout::Sampled)?;
            }
        }
        self.rects = up;
        self.shown = health;
        Ok(())
    }

    /// the HUD's pass over the image (it ends in Present)
    fn record<'a>(&'a self, e: &mut CommandEncoder<'a>, view: &'a TextureView) {
        e.begin_render(&RenderPassDesc {
            color: &[ColorAttachment { view, load: Load::Keep, clear: [0.0; 4], store: true, before: Layout::ColorTarget, after: Layout::Present }],
            depth: None,
        });
        if let Some((_, g, ..)) = &self.tex {
            e.set_render_pipeline(&self.pipe);
            e.set_bind_group(0, g);
            for r in &self.rects {
                e.set_scissor(r[0] as u32, r[1] as u32, r[2] as u32, r[3] as u32);
                e.draw(3, 1, 0, 0);
            }
        }
        e.end_render();
    }
}

fn clip(r: [i32; 4], w: i32, h: i32) -> [i32; 4] {
    let (x0, y0) = (r[0].max(0), r[1].max(0));
    let (x1, y1) = ((r[0] + r[2]).min(w), (r[1] + r[3]).min(h));
    [x0, y0, (x1 - x0).max(0), (y1 - y0).max(0)]
}

fn mode_name(m: PresentMode) -> &'static str {
    match m {
        PresentMode::Fifo => "fifo",
        PresentMode::Mailbox => "mailbox",
        PresentMode::Immediate => "immediate",
    }
}

/// path (b) measured (the module's header) -> the exit code
pub(crate) fn run() -> i32 {
    plat::setenv("jfgui_TEST_NOACTIVATE", Some("1"));
    plat::setenv("jfgui_GPU_PRESENT", None);
    let modes: Vec<PresentMode> = match arg("--present").as_deref().unwrap_or("all") {
        "fifo" => alloc::vec![PresentMode::Fifo],
        "mailbox" => alloc::vec![PresentMode::Mailbox],
        "immediate" => alloc::vec![PresentMode::Immediate],
        _ => alloc::vec![PresentMode::Fifo, PresentMode::Mailbox, PresentMode::Immediate],
    };
    let limit: f64 = arg("--limit").and_then(|v| v.parse().ok()).unwrap_or(0.0);
    let period = if limit > 0.0 { (1e6 / limit) as u64 } else { 0 };
    let fullscreen = has("--fullscreen");
    let w = plat::win_create("Spiel-Demo: eigene Swapchain", 1280, 800, plat::PWF_SYSFRAME | plat::PWF_NOACTIVATE, plat::PW_NOOWNER, plat::PW_DEFAULT, plat::PW_DEFAULT);
    if w < 0 {
        plat::write_stderr(b"gamedemo --bench own: no window\n");
        return 1;
    }
    if let Some(a) = plat::win_work_area(w) {
        // (the same place every run as path (a): the top left of the work area)
        plat::win_set_rect(w, a[0] + 8, a[1] + 40, 1280.min(a[2] - 16), 800.min(a[3] - 48));
    }
    plat::win_show(w, plat::PWS_SHOWNA);
    if fullscreen {
        plat::win_cmd(w, plat::WCMD_FULLSCREEN);
    }
    let r = (|| -> Result<()> {
        let inst = Instance::new()?;
        let surf = Surface::new(&inst, w)?;
        let adapter = inst.adapters()?.into_iter().find(|a| a.can_present(&surf)).ok_or(GpuError::NoAdapter)?;
        let dev = adapter.create_device(&DeviceDesc { max_level: Level::Voll })?;
        let mut sc = dev.create_swapchain(&surf, &SwapchainDesc { present: modes[0], frames_in_flight: 2, ..SwapchainDesc::DEFAULT })?;
        let mut g = crate::Gpu3d::new(&dev, sc.format())?;
        let mut hud = HudGpu::new(&dev, sc.format())?;
        let mut bridge = Bridge::new();
        let mut v = View::new(game::assets::Loading::default_for_target(), None);
        v.game.res.input.set_require_lock(false);
        let name = adapter.info().name.clone();
        for &mode in &modes {
            sc.set_present_mode(mode);
            let label = format!("path=b present={} window={} limit={}", mode_name(mode), if fullscreen { "full" } else { "1280x800" }, limit);
            let mut plan = Plan::new(true, label);
            v.probe = Some(plan.probe.clone());
            let (mut last, mut next, mut look_at, mut frames) = (0u64, 0u64, 0u64, 0u64);
            loop {
                // the window's events
                plat::wait(0);
                let mut closed = false;
                while let Some(ev) = plat::evq_pop() {
                    if ev.typ == plat::E_CLOSE {
                        closed = true;
                    }
                    if ev.typ == plat::E_KEY_DOWN && plat::key_code(ev.x as u32) == plat::KC_F9 && ev.data >> 31 == 0 {
                        v.presses += 1;
                        plan.probe.seen();
                    }
                    v.game.res.input.plat_event(&ev);
                }
                if closed {
                    return Ok(());
                }
                let now = plat::time_us();
                if now >= look_at {
                    look_at = now + 250_000;
                    let cw = plat::win_get_rect(w).map_or(1280, |r| r[2]);
                    plan.probe.locate(w, cw / 2, 12, false);
                }
                let step = plan.pass(now, v.presses);
                let hud_on = match step {
                    Next::Done => break,
                    Next::Wait(h) | Next::Run(h) => h != Hud::None,
                };
                // the frame limiter: wait until the frame is due (then the input is taken: late, fresh)
                if period != 0 && next > now {
                    while plat::time_us() + 1500 < next {
                        plat::sleep_ms(1);
                    }
                    while plat::time_us() < next {
                        core::hint::spin_loop();
                    }
                    continue;
                }
                if period != 0 {
                    next = if next == 0 || now > next + period { now + period } else { next + period };
                }
                let dt = if last == 0 { 0.0 } else { now.saturating_sub(last) as f64 * 1e-6 };
                last = now;
                v.cam.look(-TURN * dt, 0.0);
                v.game.res.yaw = v.cam.yaw;
                let n = v.game.clock.advance_to(now.saturating_mul(1000));
                for _ in 0..n {
                    v.game.res.tick = v.game.res.input.tick();
                    v.game.step();
                }
                v.game.res.input.end_frame();
                let alpha = v.game.alpha();
                let feet = v.game.world.get::<Pose>(v.game.res.walker).map_or(Vec3::ZERO, |p| p.interpolate(alpha).pos);
                v.cam.eye = feet + Vec3::new(0.0, EYE, 0.0);
                // the image (FIFO: acquire waits while every image is queued)
                let Some(fr) = sc.acquire()? else {
                    plat::sleep_ms(10);
                    continue;
                };
                let size = fr.size();
                let tick = v.game.world.tick();
                if hud_on {
                    hud.update(&dev, size, 100 - (tick / 60 % 100) as i32)?;
                }
                let sky = v.sky_next();
                v.assets_frame();
                let changes = core::mem::take(&mut v.changes);
                g.prepare(&mut bridge, &v.game.world, &v.cam, alpha, size, &v.assets, &mut v.models, &changes)?;
                let mut e = dev.encoder()?;
                let after = if hud_on { Layout::ColorTarget } else { Layout::Present };
                g.r.record(&mut e, &g.s, ColorAttachment { view: sc.view(&fr), load: Load::Clear, clear: sky, store: true, before: Layout::Undefined, after }, None)?;
                if hud_on {
                    hud.record(&mut e, sc.view(&fr));
                }
                let id = sc.present(fr, [e.finish()?])?;
                v.drawn();
                g.s.submitted(id);
                frames += 1;
            }
            v.probe = None;
            plat::write_stdout(format!("path (b): present {} (asked {}), adapter {}, {} x {}, frames {}\n", mode_name(sc.present_mode()), mode_name(mode), name,
                                       sc.size().0, sc.size().1, frames).as_bytes());
        }
        dev.wait_idle()?;
        Ok(())
    })();
    plat::win_destroy(w);
    match r {
        Ok(()) => 0,
        Err(e) => {
            plat::write_stderr(format!("gamedemo --bench own: {e}\n").as_bytes());
            1
        }
    }
}
