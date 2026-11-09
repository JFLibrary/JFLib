// bench -- render3dtest --bench N: the scene of gputest --bench over the engine (docs/GPU.md "Messszene";
// docs/RENDER3D.md "Messwerte")
//
//   render3dtest --bench N [--res 1080p|4k|720p|WxH] [--frames F] [--warmup W] [--inflight K] [--basis]
//                [--moved K] [--meshes M] [--verify] [--validate]
//
// The same cubes (shader::bench::move_all at time 0), the same camera path, offscreen, an Rgba8Unorm and a
// Depth32Float target, 300 frames after 60, 3 frames in flight, the same statistics.  What differs: the
// cubes stand still (the engine has no shader of its own to move them: G5a takes changes from the CPU, and
// --moved K sets K of them anew per frame); a material per cube comes from a table of 1024 colours; reverse Z.
// --meshes M: M copies of the cube's mesh, a list each.  --basis: a device limited to Basis (one indirect
// draw per list instead of one multi-draw).  --verify: the last frame's lists against shader::bench::cull_ref.
// --textures T (G5b): T textures of --tex-size S texels (256), sRGB, repeating and trilinear, the 1024
// materials take them in turn; the cube's faces get uv 0 ..= 1.
//
//   render3dtest --bench-upload              per class: the CPU's mip levels (render3d::mip_chain), add_texture
//                                            and the GPU's copies until done, for exact and resampled sizes
//   render3dtest --bench-gltf [--nodes N] [--vertices V]   a .glb made here (N nodes over 4 meshes, or one mesh
//                                            of V vertices): writing, gltf::load, the process's memory before
//                                            and after, gltf::upload, Scene::add

use alloc::format;
use alloc::vec;
use alloc::vec::Vec;

use gpu::*;
use render3d::{Camera, InstanceId, Material, MaterialId, MeshData, MeshId, Renderer, RendererDesc, Sampling, Scene, Sun, TextureData, TextureRef, Transform};
use shader::bench as sb;

use crate::common::Rng;
use crate::tests::{T, out, setup};

#[derive(Clone, Copy, Default)]
struct Stat {
    med: f64,
    p99: f64,
    mean: f64,
}

fn stat(v: &[f64]) -> Stat {
    if v.is_empty() {
        return Stat::default();
    }
    let mut s = v.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap_or(core::cmp::Ordering::Equal));
    Stat { med: s[s.len() / 2], p99: s[(s.len() * 99 / 100).min(s.len() - 1)], mean: s.iter().sum::<f64>() / s.len() as f64 }
}

/// the camera of gputest --bench at `time`, with reverse Z
fn bench_camera(time: f32, extent: f32, aspect: f64) -> Camera {
    let (s, c) = sb::sincos_turns(time * 0.02);
    let (r, h) = (extent * 0.5, extent * 0.08);
    let eye = [r * c, h, r * s];
    let (s30, c30) = (0.5f32, 0.866_025_4f32);
    let dir = [-c * c30 - s * s30, 0.0, -s * c30 + c * s30];
    let target = [eye[0] + dir[0] * extent, 0.0, eye[2] + dir[2] * extent];
    Camera::look_at([eye[0] as f64, eye[1] as f64, eye[2] as f64], [target[0] as f64, target[1] as f64, target[2] as f64], [0.0, 1.0, 0.0],
                    1.047_197_6, aspect, 0.1, extent as f64 * 3.0)
}

/// the same camera for shader::bench::cull_ref
fn g1_camera(time: f32, extent: f32, aspect: f32) -> sb::Camera {
    let (s, c) = sb::sincos_turns(time * 0.02);
    let (r, h) = (extent * 0.5, extent * 0.08);
    let eye = [r * c, h, r * s];
    let (s30, c30) = (0.5f32, 0.866_025_4f32);
    let dir = [-c * c30 - s * s30, 0.0, -s * c30 + c * s30];
    let target = [eye[0] + dir[0] * extent, 0.0, eye[2] + dir[2] * extent];
    sb::Camera::look_at(eye, target, 1.047_197_6, aspect, 0.1, extent * 3.0)
}

fn time_of(f: usize) -> f32 {
    1.0 + f as f32 / 60.0
}

pub fn bench(n: u32, validate: bool) -> i32 {
    let mut t = T { fails: 0, validate };
    let num = |name: &str, d: usize| plat::arg_value(name).and_then(|v| v.parse::<usize>().ok()).unwrap_or(d);
    let (mut w, mut h) = (1920u32, 1080u32);
    if let Some(r) = plat::arg_value("--res") {
        (w, h) = match r.as_str() {
            "1080p" => (1920, 1080),
            "4k" => (3840, 2160),
            "720p" => (1280, 720),
            s => {
                let mut p = s.split('x').filter_map(|x| x.parse::<u32>().ok());
                (p.next().unwrap_or(1920), p.next().unwrap_or(1080))
            }
        };
    }
    let (frames, warmup, k) = (num("--frames", 300), num("--warmup", 60), num("--inflight", 3).clamp(1, 3));
    let (moved, meshes) = (num("--moved", 0), num("--meshes", 1).max(1));
    let (textures, tex_size) = (num("--textures", 0), num("--tex-size", 256).clamp(1, 4096) as u32);
    // (G5c) the sun's shadows: --shadows C cascades of --shadow-size S up to --shadow-distance D, --shadow-mask 0: no
    // fragment shader in the shadow passes
    let shadows = num("--shadows", 0).min(render3d::CASCADES_MAX);
    let shadow = render3d::Shadows { cascades: shadows.max(1) as u32, size: num("--shadow-size", 2048) as u32, distance: num("--shadow-distance", 100) as f64,
                                     ..render3d::Shadows::DEFAULT };
    let shadow_mask = num("--shadow-mask", 1) != 0;
    // (G5c) --lights N point lights spread over the scene, range an eighth of its extent
    let lights = num("--lights", 0);
    // (G5c) --env 1: the procedural sky lights the scene
    let env = num("--env", 0) != 0;
    // (G5d) --occlusion 0: without the camera's occlusion test
    let occlusion = num("--occlusion", 1) != 0;
    let args = plat::args();
    let (basis, verify) = (args.iter().any(|a| a == "--basis"), args.iter().any(|a| a == "--verify"));
    let Some(g) = setup(&mut t) else { return (t.fails > 0) as i32 };
    let desc = DeviceDesc { max_level: if basis { Level::Basis } else { Level::Voll } };
    // (G5d) --compute-queue 1: the device asks for a compute queue (DeviceOptions); work beside the frame on the
    // compute queue (--async-work N, --async-on compute) asks for it too
    let async_on_compute = num("--async-work", 0) > 0 && plat::arg_value("--async-on").is_none_or(|v| v != "main");
    let opts = if num("--compute-queue", 0) != 0 || async_on_compute { DeviceOptions::DEFAULT.with_compute_queue() } else { DeviceOptions::DEFAULT };
    let Some(dev) = t.res(g.adapter.create_device_with(&desc, &opts), "device") else { return 1 };
    out(&format!("{} ({}), level {:?}{}", dev.info().name, dev.info().driver_info, dev.level(),
                 if dev.compute_queue().is_some() { ", a compute queue" } else { "" }));
    let sh = if shadows > 0 { Some((shadow, shadow_mask)) } else { None };
    if run(&mut t, &dev, n, w, h, frames, warmup, k, moved, meshes, verify, textures, tex_size, sh, lights, env, occlusion).is_none() && t.fails == 0 {
        t.ok(false, "the run stopped");
    }
    if validate {
        let (e, v) = gpu::validation_messages();
        t.ok(e == 0, &format!("validation layer: {e} errors, {v} warnings"));
    }
    (t.fails > 0) as i32
}

#[allow(clippy::too_many_arguments)]
fn run(t: &mut T, dev: &Device, n: u32, w: u32, h: u32, frames: usize, warmup: usize, k: usize, moved: usize, meshes: usize, verify: bool, textures: usize, tex_size: u32,
       shadows: Option<(render3d::Shadows, bool)>, lights: usize, env: bool, occlusion: bool) -> Option<()> {
    let extent = sb::extent_for(n);
    let push = sb::Push { time: 0.0, count: n, seed: 1, extent, object_index: 0 };
    let objs = sb::move_all(&push);
    let t0 = plat::time_us();
    let mut r = t.res(Renderer::new(dev, &RendererDesc { color: Format::Rgba8Unorm, frames_in_flight: k }), "Renderer::new")?;
    // the cube of the scene, as vertex streams
    let cube = sb::cube_vertices();
    let pos: Vec<[f32; 3]> = cube.iter().map(|v| [v.pos[0], v.pos[1], v.pos[2]]).collect();
    let nrm: Vec<[f32; 3]> = cube.iter().map(|v| [v.normal[0], v.normal[1], v.normal[2]]).collect();
    // (four corners per face: uv 0 ..= 1 over each)
    let uv: Vec<[f32; 2]> = (0..pos.len()).map(|i| [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]][i % 4]).collect();
    let idx: Vec<u32> = sb::cube_indices().iter().map(|&v| v as u32).collect();
    // (G5d) --ball S: balls of S x 2S segments in place of the cubes; --lods: their detail levels, balls of S/2 and S/4
    // below 0.02 and 0.008 of the view's height; --min-px P: nothing below P pixels (contribution culling)
    let ball_n = plat::arg_value("--ball").and_then(|v| v.parse::<u32>().ok()).unwrap_or(0);
    let lods = plat::args().iter().any(|a| a == "--lods");
    let min_px = plat::arg_value("--min-px").and_then(|v| v.parse::<f32>().ok()).unwrap_or(0.0);
    let geo = |g: &crate::common::Geo| -> (Vec<[f32; 3]>, Vec<[f32; 3]>, Vec<[f32; 2]>, Vec<u32>) { (g.pos.clone(), g.nrm.clone(), g.uv.clone(), g.idx.clone()) };
    let (pos, nrm, uv, idx) = if ball_n > 0 { geo(&crate::common::ball(ball_n, 2 * ball_n)) } else { (pos, nrm, uv, idx) };
    let mut bases = Vec::new();
    for _ in 0..meshes {
        bases.push(t.res(r.add_mesh(&MeshData { positions: &pos, normals: &nrm, uvs: &uv, indices: &idx }), "add_mesh")?);
    }
    let mut levels = Vec::new();
    if lods && ball_n > 0 {
        for (s, below) in [((ball_n / 2).max(2), 0.02f32), ((ball_n / 4).max(2), 0.008)] {
            let (p, nn, u, i) = geo(&crate::common::ball(s, 2 * s));
            let m = t.res(r.add_mesh(&MeshData { positions: &p, normals: &nn, uvs: &u, indices: &i }), "add_mesh")?;
            levels.push(render3d::Lod { mesh: Some(m), below });
        }
    }
    if min_px > 0.0 {
        levels.push(render3d::Lod { mesh: None, below: min_px / h as f32 });
    }
    if !levels.is_empty() {
        for &b in &bases {
            t.res(r.set_lods(b, &levels), "set_lods")?;
        }
        out(&format!("  detail levels: {}", levels.iter().map(|l| format!("{} below {}", l.mesh.map_or("nothing".into(), |m| format!("mesh {}", m.0)), l.below)).collect::<Vec<_>>().join(", ")));
    }
    if ball_n > 0 {
        out(&format!("  balls of {} x {} segments ({} triangles) in place of the cubes", ball_n, 2 * ball_n, idx.len() / 3));
    }
    let tt = plat::time_us();
    let mut tex = Vec::new();
    for i in 0..textures {
        let px = crate::common::noise_rgba(tex_size, tex_size, 1000 + i as u32);
        tex.push(t.res(r.add_texture(&TextureData { width: tex_size, height: tex_size, rgba: &px, srgb: true, nearest: false }), "add_texture")?);
    }
    if textures > 0 {
        let id = t.res(dev.queue().flush(), "flush")?;
        t.res(dev.wait(id, 60_000_000_000), "wait")?;
        out(&format!("  {textures} textures of {tex_size} x {tex_size} with their levels: {:.0} ms until the GPU has them, {} MiB of arrays", (plat::time_us() - tt) as f64 / 1000.0,
                     r.texture_bytes() >> 20));
    }
    for (i, o) in objs.iter().take(1024).enumerate() {
        let mut m = Material::color(o.color[0], o.color[1], o.color[2]);
        if !tex.is_empty() {
            m.base_color_texture = Some(TextureRef { texture: tex[i % tex.len()], sampling: Sampling::LINEAR });
        }
        t.res(r.add_material(&m), "add_material")?;
    }
    let palette = objs.len().min(1024) as u32;
    let mut s = t.res(Scene::new(&r), "Scene::new")?;
    let amb = 0.25f32;
    s.set_sun(&Sun { direction: [0.3, 0.8, 0.5], color: [(1.0 - amb) * core::f32::consts::PI; 3], ambient: [amb; 3] });
    if env {
        t.res(r.set_environment(&render3d::EnvironmentData::Sky(render3d::Sky::DEFAULT)), "set_environment")?;
        s.set_environment(Some(1.0));
        out("  the procedural sky lights the scene");
    }
    if lights > 0 {
        let mut lr = Rng(321);
        let e = extent as f64 * 0.5;
        let list: Vec<render3d::Light> = (0..lights).map(|_| render3d::Light::point([lr.range(-e, e), lr.range(-e * 0.2, e * 0.2), lr.range(-e, e)],
                                                                                      [lr.range(0.3, 1.0) as f32, lr.range(0.3, 1.0) as f32, lr.range(0.3, 1.0) as f32],
                                                                                      (e * e * 0.01) as f32, Some(extent * 0.125))).collect();
        t.res(s.set_lights(&list), "set_lights")?;
        out(&format!("  {lights} point lights over the scene, range {:.0}", extent * 0.125));
    }
    if let Some((sh, mask)) = &shadows {
        t.res(s.set_shadows(Some(sh)), "set_shadows")?;
        r.set_shadow_mask(*mask);
        out(&format!("  shadows: {} cascades of {} up to {} (scene extent {extent:.0}), the shadow pass {}", sh.cascades, sh.size, sh.distance,
                     if *mask { "with fs_shadow (MASK's holes)" } else { "depth only" }));
    }
    let mut ids: Vec<InstanceId> = Vec::with_capacity(n as usize);
    for (i, o) in objs.iter().enumerate() {
        let transform = Transform { pos: [o.pos[0] as f64, o.pos[1] as f64, o.pos[2] as f64], rot: o.rot, scale: [o.scale; 3] };
        ids.push(t.res(s.add(render3d::Instance { mesh: MeshId(i as u32 % meshes as u32), material: MaterialId(i as u32 % palette), transform }), "add")?);
    }
    let t_add = (plat::time_us() - t0) as f64 / 1000.0;
    s.set_stats(true);
    s.set_occlusion(occlusion);
    let color = t.res(dev.create_texture(&TextureDesc::d2(w, h, Format::Rgba8Unorm, TextureUsage::COLOR_TARGET | TextureUsage::COPY_SRC)), "colour target")?;
    // (G5d: sampled, the occlusion test's pyramid reads it)
    let depth = t.res(dev.create_texture(&TextureDesc::d2(w, h, Format::Depth32Float, TextureUsage::DEPTH_TARGET | TextureUsage::SAMPLED)), "depth target")?;
    let aspect = w as f64 / h as f64;
    let total = warmup + frames;
    let (mut rec, mut cpu, mut wait, mut prep, mut interval) = (vec![0.0; total], vec![0.0; total], vec![0.0; total], vec![0.0; total], vec![0.0; total]);
    let mut rng = Rng(123);
    let mut last = plat::time_us();
    let mut first_frame = 0.0;
    // (G5d) --async-work N --async-on compute|main: beside every frame render3d's scatter shader puts N changes into a
    // second copy of the instance data (what scattering the next frame's changes into double-buffered data would
    // cost), on the compute queue or after the frame on the device queue: does it run beside the drawing?
    let async_n = plat::arg_value("--async-work").and_then(|v| v.parse::<u32>().ok()).unwrap_or(0);
    let on_compute = plat::arg_value("--async-on").is_none_or(|v| v != "main");
    let side = if async_n > 0 { Some(t.res(Side::new(dev, n, async_n), "async work")?) } else { None };
    if side.is_some() {
        out(&format!("  beside every frame: render3d's scatter shader with {async_n} changes into a second copy of the instance data, on {}",
                     if on_compute && dev.compute_queue().is_some() { "the compute queue" } else { "the device queue after the frame" }));
    }
    let mut side_ids: Vec<SubmitId> = Vec::new();
    for f in 0..total {
        let tw = plat::time_us();
        t.res(s.wait_slot(), "wait")?;
        let t0 = plat::time_us();
        s.set_camera(&bench_camera(time_of(f), extent, aspect));
        for _ in 0..moved {
            let id = ids[rng.below(n) as usize];
            let mut tr = t.res(s.transform(id), "transform")?;
            tr.pos[1] += if rng.next() & 1 == 0 { 0.01 } else { -0.01 };
            t.res(s.set_transform(id, &tr), "set_transform")?;
        }
        t.res(s.prepare(&mut r), "prepare")?;
        let tp = plat::time_us();
        let mut e = t.res(dev.encoder(), "encoder")?;
        let first = f == 0;
        let ca = ColorAttachment { view: color.view(), load: Load::Clear, clear: [0.05, 0.06, 0.09, 1.0], store: true,
                                   before: if first { Layout::Undefined } else { Layout::ColorTarget }, after: Layout::ColorTarget };
        let da = DepthAttachment { view: depth.view(), load: Load::Clear, clear: render3d::DEPTH_CLEAR, store: false,
                                   before: if first { Layout::Undefined } else { Layout::DepthTarget }, after: Layout::DepthTarget };
        t.res(r.record(&mut e, &s, ca, Some(da)), "record")?;
        let cmd = t.res(e.finish(), "finish")?;
        let tr = plat::time_us();
        let id = t.res(dev.queue().submit([cmd]), "submit")?;
        let t1 = plat::time_us();
        s.submitted(id);
        if let Some(sd) = &side {
            // (at most three in flight, like the frames)
            if side_ids.len() >= 3 {
                let old = side_ids.remove(0);
                t.res(dev.wait(old, 10_000_000_000), "wait")?;
            }
            side_ids.push(t.res(sd.run(dev, on_compute), "async work")?);
        }
        wait[f] = (t0 - tw) as f64 / 1000.0;
        prep[f] = (tp - t0) as f64 / 1000.0;
        rec[f] = (tr - tp) as f64 / 1000.0;
        cpu[f] = (t1 - t0) as f64 / 1000.0;
        interval[f] = (t1 - last) as f64 / 1000.0;
        if f == 0 {
            first_frame = prep[0];
        }
        last = t1;
    }
    t.res(dev.wait_idle(), "wait_idle")?;
    s.collect_stats();
    let st = s.take_stats();
    let m: Vec<_> = st.iter().filter(|x| x.frame as usize >= warmup).collect();
    let pick = |f: &dyn Fn(&render3d::FrameStats) -> Option<f64>| -> Stat { stat(&m.iter().filter_map(|x| f(x)).collect::<Vec<f64>>()) };
    let vis = pick(&|x| Some(x.visible as f64));
    // (G5d) phase 2's instances, the pyramid, phase 2's culling and pass
    let late = pick(&|x| Some(x.late as f64));
    let hiz: Vec<Stat> = (0..3).map(|k| pick(&|x| x.stamps.map(|_| x.occlusion[k]))).collect();
    let up = pick(&|x| x.stamps.map(|a| a[1] - a[0]));
    let cull = pick(&|x| x.stamps.map(|a| a[2] - a[1]));
    let draw = pick(&|x| x.stamps.map(|_| x.draw));
    // (G5c) the cascades' culling and depth passes, summed and each
    let shcull = pick(&|x| x.stamps.map(|_| x.cascades.iter().map(|c| c[0]).sum()));
    let shdraw = pick(&|x| x.stamps.map(|_| x.cascades.iter().map(|c| c[1]).sum()));
    let per: Vec<(Stat, Stat)> = (0..render3d::CASCADES_MAX).map(|k| (pick(&|x| x.stamps.map(|_| x.cascades[k][0])), pick(&|x| x.stamps.map(|_| x.cascades[k][1])))).collect();
    let all = pick(&|x| x.stamps.map(|a| a[3] - a[0]));
    let mut gaps = Vec::new();
    for p in m.windows(2) {
        if let (Some(a), Some(b)) = (p[0].stamps, p[1].stamps) {
            gaps.push(b[3] - a[3]);
        }
    }
    let gap = stat(&gaps);
    let busy: f64 = m.iter().skip(1).filter_map(|x| x.stamps.map(|a| a[3] - a[0])).sum();
    let busy = if gaps.is_empty() { 0.0 } else { busy / gaps.iter().sum::<f64>() };
    let (rs, cs, ws, ps, fs) = (stat(&rec[warmup..]), stat(&cpu[warmup..]), stat(&wait[warmup..]), stat(&prep[warmup..]), stat(&interval[warmup..]));
    let info = s.upload_info();
    out(&format!("scene: {n} cubes in {} lists, {w}x{h} offscreen, {k} frames in flight, {frames} frames after {warmup} warm-up, {moved} moved per frame, {}",
                 meshes, if r.multi_draw() { "one multi-draw" } else { "one indirect draw per list" }));
    out(&format!("  building: {t_add:.0} ms ({n} adds on the CPU), the first prepare {first_frame:.0} ms (all of it into staging); blocks of GPU and staging memory after the run: {:.0} MiB", dev.memory_reserved() as f64 / 1048576.0));
    out(&format!("  visible: {:.0} cubes (median) = {:.1} %; last frame sent {} bytes", vis.med, vis.med / n as f64 * 100.0, info.bytes));
    out(&format!("  CPU: prepare {:.3} ms, record {:.3}, prepare + record + submit {:.3} median, {:.3} p99; waiting for the GPU {:.3}", ps.med, rs.med, cs.med, cs.p99, ws.med));
    out(&format!("  GPU: upload {:.3} / cull {:.3} / draw {:.3} / total {:.3} ms median; p99 {:.3} / {:.3} / {:.3} / {:.3}", up.med, cull.med, draw.med, all.med, up.p99, cull.p99, draw.p99, all.p99));
    out(&format!("  GPU frame interval {:.3} ms median, {:.3} p99; GPU busy {:.1} %", gap.med, gap.p99, busy * 100.0));
    if shadows.is_some() {
        out(&format!("  cascades (culling / depth pass, ms median): {}", per.iter().map(|(c, d)| format!("{:.3} / {:.3}", c.med, d.med)).collect::<Vec<_>>().join("; ")));
    }
    if occlusion {
        out(&format!("  occlusion test: {:.0} drawn by phase 2 (median); pyramid {:.3} / phase 2's culling {:.3} / phase 2's pass {:.3} ms median (in draw: both passes)",
                     late.med, hiz[0].med, hiz[1].med, hiz[2].med));
    }
    let fps = if fs.mean > 0.0 { 1000.0 / fs.mean } else { 0.0 };
    out(&format!("  frame: {:.3} ms mean ({fps:.0} fps), {:.3} median, {:.3} p99", fs.mean, fs.med, fs.p99));
    out(&format!("RESULT engine n={n} res={w}x{h} lists={meshes} moved={moved} textures={textures}x{tex_size} level={:?} inflight={k} frames={} occl={} ball={ball_n} lods={lods} minpx={min_px} vis={:.4} late={:.4} prep={:.4}/{:.4} rec={:.4}/{:.4} cpu={:.4}/{:.4} wait={:.4}/{:.4} up={:.4}/{:.4} cull={:.4}/{:.4} shcull={:.4}/{:.4} shdraw={:.4}/{:.4} draw={:.4}/{:.4} hiz={:.4}/{:.4}/{:.4} gpu={:.4}/{:.4} gpuframe={:.4}/{:.4} frame={:.4}/{:.4} framemean={:.4} busy={:.3}",
                 dev.level(), m.len(), occlusion as u32, vis.med / n as f64, late.med / n as f64, ps.med, ps.p99, rs.med, rs.p99, cs.med, cs.p99, ws.med, ws.p99, up.med, up.p99, cull.med, cull.p99,
                 shcull.med, shcull.p99, shdraw.med, shdraw.p99, draw.med, draw.p99, hiz[0].med, hiz[1].med, hiz[2].med, all.med, all.p99, gap.med, gap.p99, fs.med, fs.p99, fs.mean, busy));
    if verify {
        // (G5d) the lists of a frame without the occlusion test at the last frame's camera: everything in the view
        if occlusion {
            s.set_occlusion(false);
            s.set_camera(&bench_camera(time_of(total - 1), extent, aspect));
            t.res(s.prepare(&mut r), "prepare")?;
            let mut e = t.res(dev.encoder(), "encoder")?;
            let ca = ColorAttachment { view: color.view(), load: Load::Clear, clear: [0.05, 0.06, 0.09, 1.0], store: true, before: Layout::ColorTarget, after: Layout::ColorTarget };
            let da = DepthAttachment { view: depth.view(), load: Load::Clear, clear: render3d::DEPTH_CLEAR, store: false, before: Layout::DepthTarget, after: Layout::DepthTarget };
            t.res(r.record(&mut e, &s, ca, Some(da)), "record")?;
            let cmd = t.res(e.finish(), "finish")?;
            let id = t.res(dev.queue().submit([cmd]), "submit")?;
            s.submitted(id);
            t.res(dev.wait_idle(), "wait_idle")?;
        }
        let in_view = verify_lists(t, dev, &s, &objs, ids.len(), time_of(total - 1), extent, w, h)?;
        if occlusion {
            out(&format!("  with the occlusion test {:.0} of {in_view} in the view drawn (median, {:.1} %)", vis.med, vis.med / in_view.max(1) as f64 * 100.0));
        }
    }
    Some(())
}

/// (G5d) the work beside the frames: render3d's scatter shader over its own buffers
struct Side {
    pipe: ComputePipeline,
    group: BindGroup,
    n: u32,
}

impl Side {
    fn new(dev: &Device, cap: u32, n: u32) -> gpu::Result<Side> {
        let m = ShaderModule::from_spirv(dev, render3d::SCATTER_SPV)?;
        let b = |binding, kind| Binding { binding, kind, stages: Stages::COMPUTE };
        let bgl = dev.create_bind_group_layout(&[b(0, BindingKind::StorageBufferRead), b(1, BindingKind::StorageBuffer), b(2, BindingKind::StorageBuffer),
                                                 b(3, BindingKind::StorageBuffer), b(4, BindingKind::StorageBuffer)])?;
        let pl = dev.create_pipeline_layout(&PipelineLayoutDesc { groups: &[&bgl], push_constants: 4 })?;
        let pipe = dev.create_compute_pipeline(&ComputePipelineDesc { layout: &pl, shader: ShaderStage::new(&m, "main") })?;
        let buf = |size: u64, memory| dev.create_buffer(&BufferDesc { size, usage: BufferUsage::STORAGE, memory });
        // changes of 96 bytes: slot, list, 2 reserved, the sphere, the instance's 16 words
        let mut rng = Rng(77);
        let rows: Vec<[u32; 24]> = (0..n).map(|_| {
            let mut r = [0u32; 24];
            r[0] = rng.below(cap);
            r[7] = 1.0f32.to_bits();
            r
        }).collect();
        let changes = buf(n as u64 * 96, MemoryKind::Upload)?;
        changes.write(0, &rows)?;
        let (spheres, list_of, inst, hidden) = (buf(cap as u64 * 16, MemoryKind::Device)?, buf(cap as u64 * 4, MemoryKind::Device)?, buf(cap as u64 * 64, MemoryKind::Device)?,
                                                buf(cap.div_ceil(32) as u64 * 4, MemoryKind::Device)?);
        let e = |binding, buffer| BindEntry { binding, resource: Resource::Buffer { buffer, offset: 0, size: 0 } };
        let group = dev.create_bind_group(&BindGroupDesc { layout: &bgl, entries: &[e(0, &changes), e(1, &spheres), e(2, &list_of), e(3, &inst), e(4, &hidden)] })?;
        Ok(Side { pipe, group, n })
    }

    fn run(&self, dev: &Device, on_compute: bool) -> gpu::Result<SubmitId> {
        let cq = dev.compute_queue().filter(|_| on_compute);
        let mut e = match cq {
            Some(q) => q.encoder()?,
            None => dev.encoder()?,
        };
        // the run before wrote the same buffers
        e.barrier(Access::COMPUTE_WRITE, Access::COMPUTE_WRITE);
        e.set_compute_pipeline(&self.pipe);
        e.set_bind_group(0, &self.group);
        e.push_constants(0, &[self.n]);
        e.dispatch(self.n.div_ceil(64), 1, 1);
        let cmd = e.finish()?;
        match cq {
            Some(q) => q.submit([cmd]),
            None => dev.queue().submit([cmd]),
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn verify_lists(t: &mut T, dev: &Device, s: &Scene, objs: &[sb::Object], n: usize, time: f32, extent: f32, w: u32, h: u32) -> Option<usize> {
    let cam = g1_camera(time, extent, w as f32 / h as f32);
    let want = sb::cull_ref(objs, &cam);
    let lists = s.lists();
    let mut draws = vec![0u32; lists.len() * 5];
    t.res(dev.queue().read_buffer(s.draws_buffer(), 0, &mut draws), "read draws")?;
    let mut in_list = vec![false; n];
    let mut got = 0;
    for (i, l) in lists.iter().enumerate() {
        let c = draws[i * 5 + 1] as usize;
        let mut v = vec![0u32; c];
        if c > 0 {
            t.res(dev.queue().read_buffer(s.visible_buffer(), l.first as u64 * 4, &mut v), "read visible")?;
        }
        for x in v {
            if let Some(b) = in_list.get_mut(x as usize) {
                *b = true;
                got += 1;
            }
        }
    }
    let missing = want.iter().filter(|&&i| !in_list[i as usize]).count();
    let extra = got as i64 - want.len() as i64;
    t.ok(missing == 0, &format!("--verify: the last frame's lists hold all {} of gputest's cull_ref (at the camera it had), {} more (spheres grown for f32's rounding), {missing} missing", want.len(), extra));
    Some(got)
}

// ---------------------------------------------------------------- textures and glTF (G5b)

/// --bench-upload: per class the CPU's levels alone, add_texture, and until the GPU has them
pub fn bench_upload() -> i32 {
    let mut t = T { fails: 0, validate: false };
    let Some(g) = setup(&mut t) else { return 0 };
    let Some(dev) = t.res(g.adapter.create_device(&DeviceDesc::DEFAULT), "device") else { return 1 };
    out(&format!("{} ({}), level {:?}; release builds measure, a debug build is slower", dev.info().name, dev.info().driver_info, dev.level()));
    for k in 0..render3d::CLASSES as u32 {
        let size = render3d::CLASS_MIN << k;
        let reps = if size <= 512 { 8 } else { 4 };
        for (label, w, h, srgb) in [("sRGB", size, size, true), ("linear", size, size, false), ("sRGB resampled", size * 3 / 4, size * 5 / 8, true)] {
            let px = crate::common::noise_rgba(w, h, k * 10 + w);
            let d = TextureData { width: w, height: h, rgba: &px, srgb, nearest: false };
            let t0 = plat::time_us();
            let mut bytes = 0;
            for _ in 0..reps {
                bytes = render3d::mip_chain(&d).iter().map(|l| l.len()).sum::<usize>();
            }
            let levels = (plat::time_us() - t0) as f64 / 1000.0 / reps as f64;
            // a fresh renderer: the first add makes the array (4 layers), the next fit in
            let Some(mut r) = t.res(Renderer::new(&dev, &RendererDesc::new(Format::Rgba8Unorm)), "Renderer::new") else { return 1 };
            let Some(id) = t.res(dev.queue().flush(), "flush") else { return 1 };
            t.res(dev.wait(id, 10_000_000_000), "wait");
            let reps = reps.min(4);
            let t1 = plat::time_us();
            for _ in 0..reps {
                t.res(r.add_texture(&d), "add_texture");
            }
            let add = (plat::time_us() - t1) as f64 / 1000.0 / reps as f64;
            let Some(id) = t.res(dev.queue().flush(), "flush") else { return 1 };
            t.res(dev.wait(id, 60_000_000_000), "wait");
            let all = (plat::time_us() - t1) as f64 / 1000.0 / reps as f64;
            out(&format!("  {w} x {h} {label} -> class {size}: levels on the CPU {levels:.2} ms, add_texture {add:.2} ms, until the GPU has it {all:.2} ms ({:.1} MiB with levels)", bytes as f64 / 1048576.0));
            out(&format!("RESULT upload w={w} h={h} srgb={srgb} class={size} levels={levels:.3} add={add:.3} gpu={all:.3} bytes={bytes}"));
        }
    }
    (t.fails > 0) as i32
}

/// --bench-gltf: a .glb made here, read and put on the GPU
pub fn bench_gltf() -> i32 {
    use crate::gltfgen::{Enc, Form, GMat, GNode, GPrim, GScene};
    let mut t = T { fails: 0, validate: false };
    let num = |name: &str| plat::arg_value(name).and_then(|v| v.parse::<usize>().ok());
    let (nodes, vertices) = match (num("--nodes"), num("--vertices")) {
        (None, None) => (100_000, 0),
        (n, v) => (n.unwrap_or(0), v.unwrap_or(0)),
    };
    let enc = Enc { idx: Some(5125), uv: Some(5126), normals: true, interleaved: false, sparse: None };
    let mat = GMat { factor: [0.8, 0.7, 0.6, 1.0], metallic: 0.0, roughness: 1.0, emissive: [0.0; 3], tex: [None; 5], mask: None };
    let id = [0.0, 0.0, 0.0, 1.0];
    let mut sc = GScene { meshes: Vec::new(), mats: vec![mat], texs: Vec::new(), images: Vec::new(), nodes: Vec::new(), scenes: vec![vec![0]], scene: 0 };
    let what;
    if vertices > 0 {
        // one grid of side x side vertices
        let side = (crate::cpu::exp(crate::cpu::ln(vertices as f64) * 0.5) as usize).max(2);
        let mut geo = crate::common::Geo { pos: Vec::new(), nrm: Vec::new(), uv: Vec::new(), idx: Vec::new() };
        for y in 0..side {
            for x in 0..side {
                let (u, v) = (x as f32 / (side - 1) as f32, y as f32 / (side - 1) as f32);
                geo.pos.push([u * 100.0 - 50.0, v * 100.0 - 50.0, ((x * 7 + y * 3) % 11) as f32 * 0.01]);
                geo.nrm.push([0.0, 0.0, 1.0]);
                geo.uv.push([u, v]);
            }
        }
        for y in 0..side as u32 - 1 {
            for x in 0..side as u32 - 1 {
                let (a, b, c, d) = (y * side as u32 + x, y * side as u32 + x + 1, (y + 1) * side as u32 + x, (y + 1) * side as u32 + x + 1);
                geo.idx.extend_from_slice(&[a, b, d, a, d, c]);
            }
        }
        what = format!("one mesh of {} vertices, {} triangles", geo.pos.len(), geo.idx.len() / 3);
        sc.meshes.push(vec![GPrim { geo, enc, material: Some(0) }]);
        sc.nodes.push(GNode { t: [0.0; 3], r: id, s: [1.0; 3], matrix: false, mesh: Some(0), children: Vec::new() });
    } else {
        // a root, 100 groups, the nodes under them, over 4 meshes
        use crate::common::{ball, cube, plate, prism};
        for g in [cube(), ball(8, 12), prism(6, 0.5), plate()] {
            sc.meshes.push(vec![GPrim { geo: g, enc, material: Some(0) }]);
        }
        let groups = 100.min(nodes.max(1));
        sc.nodes.push(GNode { t: [0.0; 3], r: id, s: [1.0; 3], matrix: false, mesh: None, children: (1..=groups).collect() });
        for gi in 0..groups {
            sc.nodes.push(GNode { t: [(gi % 10) as f64 * 30.0, 0.0, (gi / 10) as f64 * 30.0], r: id, s: [1.0; 3], matrix: false, mesh: None, children: Vec::new() });
        }
        let mut rng = crate::common::Rng(17);
        for i in 0..nodes {
            let at = sc.nodes.len();
            sc.nodes[1 + i % groups].children.push(at);
            let (s, c) = crate::cpu::sincos(rng.range(0.0, 6.0) * 0.5);
            let sign = if i % 5 == 0 { -1.0 } else { 1.0 };
            sc.nodes.push(GNode { t: [rng.range(0.0, 30.0), rng.range(0.0, 10.0), rng.range(0.0, 30.0)], r: [0.0, s, 0.0, c], s: [0.3 * sign, 0.3, 0.3], matrix: false, mesh: Some(i % 4),
                                  children: Vec::new() });
        }
        what = format!("{nodes} nodes with a mesh (a fifth mirrored) under {groups} groups, 4 meshes");
    }
    let t0 = plat::time_us();
    let (glb, _) = sc.write(Form::Glb);
    let write_ms = (plat::time_us() - t0) as f64 / 1000.0;
    drop(sc);
    let mem0 = plat::win_test_usage().2;
    let t1 = plat::time_us();
    let doc = match gltf::load(&glb, &mut |_| None, &gltf::Limits::DEFAULT) {
        Ok(d) => d,
        Err(e) => {
            t.ok(false, &format!("gltf::load: {e}"));
            return 1;
        }
    };
    let load_ms = (plat::time_us() - t1) as f64 / 1000.0;
    let mem1 = plat::win_test_usage().2;
    let model: usize = doc.meshes.iter().flat_map(|m| &m.primitives).map(|p| p.positions.len() * 12 + p.normals.len() * 12 + p.uvs.len() * 8 + p.indices.len() * 4).sum::<usize>()
        + doc.nodes.len() * core::mem::size_of::<gltf::Node>() + doc.nodes.iter().map(|n| n.children.len() * 8).sum::<usize>();
    out(&format!("glTF: {what}; .glb of {:.1} MB written in {write_ms:.0} ms", glb.len() as f64 / 1e6));
    out(&format!("  gltf::load {load_ms:.1} ms ({:.0} MB/s); private memory of the process {:.1} -> {:.1} MB (+{:.1}), the model holds {:.1} MB",
                 glb.len() as f64 / 1e6 / (load_ms / 1000.0), mem0 as f64 / 1e6, mem1 as f64 / 1e6, (mem1 as f64 - mem0 as f64) / 1e6, model as f64 / 1e6));
    let Some(g) = setup(&mut t) else { return (t.fails > 0) as i32 };
    let Some(dev) = t.res(g.adapter.create_device(&DeviceDesc::DEFAULT), "device") else { return 1 };
    let Some(mut r) = t.res(Renderer::new(&dev, &RendererDesc::new(Format::Rgba8Unorm)), "Renderer::new") else { return 1 };
    let t2 = plat::time_us();
    let Some(imp) = t.res(render3d::gltf::upload(&mut r, &doc, None), "gltf::upload") else { return 1 };
    let upload_ms = (plat::time_us() - t2) as f64 / 1000.0;
    let Some(mut s) = t.res(Scene::new(&r), "Scene::new") else { return 1 };
    let t3 = plat::time_us();
    for i in &imp.instances {
        if t.res(s.add(*i), "add").is_none() {
            return 1;
        }
    }
    let add_ms = (plat::time_us() - t3) as f64 / 1000.0;
    let t4 = plat::time_us();
    t.res(s.prepare(&mut r), "prepare");
    let prep_ms = (plat::time_us() - t4) as f64 / 1000.0;
    let mirrored = s.lists().iter().filter(|l| l.mirrored).map(|l| l.count).sum::<u32>();
    out(&format!("  gltf::upload {upload_ms:.1} ms, {} instances ({mirrored} mirrored) added in {add_ms:.1} ms, the first prepare {prep_ms:.1} ms; {} lists",
                 imp.instances.len(), s.lists().len()));
    out(&format!("RESULT gltf nodes={nodes} vertices={vertices} glb={} write={write_ms:.1} load={load_ms:.2} mem={} model={model} upload={upload_ms:.2} add={add_ms:.2} prepare={prep_ms:.2}",
                 glb.len(), mem1 as i64 - mem0 as i64));
    t.res(dev.wait_idle(), "wait_idle");
    (t.fails > 0) as i32
}
