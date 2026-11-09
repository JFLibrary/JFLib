// web -- render3dtest in the browser (tests/engine_web.mjs reads the console): the engine over WebGPU, from
// the same WGSL, on the app's worker.  The gallery's five images (G5a's three, the textured one with
// mirrored instances, the glTF scene read from a .glb made here) go out as hex lines, compared with those
// of the Vulkan path on the same machine by the test; the counts of the lists against the exact test on
// the CPU.  Nothing can wait here: every frame is submitted at once, the read-backs are looked at per tick.
// (G5e2) The same gallery in an sRGB target (srgb_NAME, the hardware encodes), with the shader's sRGB in an Rgba8Unorm
// target (enc_NAME), and the latter against the same image in Rgba32Float encoded on the CPU, here in the browser.

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use gpu::*;
use render3d::{InstanceId, MeshId, Renderer, RendererDesc, Scene};

use crate::cpu::srgb_encode;

use crate::common::*;
use crate::cpu;

/// the instances the camera can see, by the exact test on the CPU: (certainly visible, in doubt)
fn exact_visible(r: &Renderer, s: &Scene, specs: &[Spec]) -> (u32, u32) {
    let c = s.camera();
    let cam = cpu::RefCamera::new(c.eye, c.forward, c.up, c.fov_y, c.aspect, c.near, c.far);
    let (mut yes, mut maybe) = (0, 0);
    for spec in specs {
        let Some(m) = r.mesh_info(MeshId(spec.mesh)) else { continue };
        let (center, radius) = cpu::world_sphere(m.center, m.radius, spec.pos, spec.rot, spec.scale);
        match cam.sphere_test(center, radius, s.origin()) {
            cpu::Seen::Yes => yes += 1,
            cpu::Seen::Doubt => maybe += 1,
            cpu::Seen::No => {}
        }
    }
    (yes, maybe)
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

/// (G5e2) the gallery's images drawn into an sRGB target too (tests/engine_web.mjs: against `--dump --format rgba8srgb`)
const SRGB_TARGET: [(&str, &str); 8] = [("mix", "srgb_mix"), ("tex", "srgb_tex"), ("gltf", "srgb_gltf"), ("pbr", "srgb_pbr"), ("shadow", "srgb_shadow"),
                                        ("lights", "srgb_lights"), ("env", "srgb_env"), ("occl", "srgb_occl")];
/// (G5e2) and with the shader's sRGB in Rgba8Unorm (against `--dump --format rgba8`), with their float images
const ENCODED: [(&str, &str, &str); 8] = [("mix", "enc_mix", "f_mix"), ("tex", "enc_tex", "f_tex"), ("gltf", "enc_gltf", "f_gltf"), ("pbr", "enc_pbr", "f_pbr"),
                                         ("shadow", "enc_shadow", "f_shadow"), ("lights", "enc_lights", "f_lights"), ("env", "enc_env", "f_env"),
                                         ("occl", "enc_occl", "f_occl")];

fn say(s: &str) {
    rt::os::write_stdout(format!("{s}\n").as_bytes());
}

/// the visible counts of a frame, read back once the browser has answered
struct Counted {
    name: &'static str,
    rb: Buffer,
    lists: usize,
    want: (u32, u32),
}

struct Probe {
    step: u32,
    inst: Option<Instance>,
    dev: Option<Device>,
    keep: Vec<(&'static str, Off)>,
    /// (G5e2) float images, not sent: compared here with the encoded ones (which `encoded` keeps once read)
    floats: Vec<(&'static str, &'static str, Off)>,
    encoded: Vec<(&'static str, Vec<u8>)>,
    scenes: Vec<Scene>,
    renderer: Option<Renderer>,
    more: Vec<Renderer>,
    counts: Vec<Counted>,
    ok: u32,
    bad: u32,
    t0: u64,
}

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
        say(&format!("render3dtest: {} ok, {} failed, {:.1} ms", self.ok, self.bad, (plat::time_us() - self.t0) as f64 / 1000.0));
        self.step = u32::MAX;
    }

    /// a frame of the gallery into a target of its own, with the lists' counts copied out
    fn frame(&mut self, dev: &Device, r: &mut Renderer, s: &mut Scene, name: &'static str, specs: &[Spec]) -> Result<()> {
        let mut off = Off::new(dev, GALLERY_W, GALLERY_H, Format::Rgba8Unorm)?;
        off.submit(r, s)?;
        let lists = s.lists().len();
        let rb = dev.create_buffer(&BufferDesc { size: lists as u64 * 20, usage: BufferUsage::NONE, memory: MemoryKind::Readback })?;
        let mut e = dev.encoder()?;
        e.copy_buffer(s.draws_buffer(), 0, &rb, 0, lists as u64 * 20);
        dev.queue().submit([e.finish()?])?;
        self.counts.push(Counted { name, rb, lists, want: exact_visible(r, s, specs) });
        self.keep.push((name, off));
        Ok(())
    }

    fn gallery(&mut self, dev: &Device) -> Result<()> {
        let mut r = Renderer::new(dev, &linear_desc(Format::Rgba8Unorm))?;
        say(&format!("renderer: multi-draw {}, at most {} instances", r.multi_draw(), r.max_instances()));
        self.yes(!r.multi_draw(), "the browser's level Basis: one indirect draw per list");
        let (meshes, materials) = gallery_fill(&mut r)?;
        let (mut s, mut ids): (Scene, Vec<InstanceId>) = gallery_scene(&r, meshes, materials, [0.0; 3])?;
        let specs = gallery_specs(meshes, materials, GALLERY_N, [0.0; 3]);
        self.frame(dev, &mut r, &mut s, "mix", &specs)?;
        self.yes(s.upload_info().full, "the first frame sends everything at once");
        gallery_change(&mut s, &mut ids, meshes, materials)?;
        let mut off = Off::new(dev, GALLERY_W, GALLERY_H, Format::Rgba8Unorm)?;
        off.submit(&mut r, &mut s)?;
        let info = s.upload_info();
        self.yes(info.changed > 0 && !info.full, &format!("the changes go through the scatter shader: {} slots", info.changed));
        self.keep.push(("changed", off));
        let far = [1e7, -3e6, 2e7];
        let (mut f, _) = gallery_scene(&r, meshes, materials, far)?;
        let specs_far = gallery_specs(meshes, materials, GALLERY_N, far);
        self.frame(dev, &mut r, &mut f, "far", &specs_far)?;
        self.scenes.push(s);
        self.scenes.push(f);
        self.renderer = Some(r);
        // G5b: textures of four classes and mirrored instances; the glTF scene
        for name in ["tex", "gltf"] {
            let mut r = Renderer::new(dev, &linear_desc(Format::Rgba8Unorm))?;
            let mut s = if name == "tex" { tex_gallery(&mut r)? } else { gltf_gallery(&mut r)? };
            let mut off = Off::new(dev, GALLERY_W, GALLERY_H, Format::Rgba8Unorm)?;
            off.submit(&mut r, &mut s)?;
            let mirrored = s.lists().iter().filter(|l| l.mirrored).count();
            self.yes(mirrored > 0 && r.texture_count() > 0, &format!("{name}: {} textures ({} MiB of arrays), {} lists of which {mirrored} mirrored", r.texture_count(),
                                                                     r.texture_bytes() >> 20, s.lists().len()));
            self.keep.push((name, off));
            self.scenes.push(s);
            self.more.push(r);
        }
        // G5c
        for name in LATER {
            let mut r = Renderer::new(dev, &linear_desc(Format::Rgba8Unorm))?;
            let mut s = later_gallery_in(name, &mut r, Format::Rgba8Unorm)?;
            let mut off = Off::new(dev, GALLERY_W, GALLERY_H, Format::Rgba8Unorm)?;
            off.submit(&mut r, &mut s)?;
            self.keep.push((name, off));
            self.scenes.push(s);
            self.more.push(r);
        }
        // (G5e2) the gallery in an sRGB target, where the hardware encodes: srgb_NAME
        for (name, tag) in SRGB_TARGET {
            let off = self.in_format(dev, name, RendererDesc::new(Format::Rgba8Srgb))?;
            self.keep.push((tag, off));
        }
        // (G5e2) the shader's sRGB, and the float image for it
        for (name, tag, f) in ENCODED {
            let off = self.in_format(dev, name, RendererDesc::new(Format::Rgba8Unorm))?;
            self.keep.push((tag, off));
            let off = self.in_format(dev, name, RendererDesc::new(Format::Rgba32Float))?;
            self.floats.push((f, tag, off));
        }
        Ok(())
    }

    /// one of the gallery's images with a renderer of this description and a target of its format
    fn in_format(&mut self, dev: &Device, name: &'static str, desc: RendererDesc) -> Result<Off> {
        let format = desc.color;
        let mut r = Renderer::new(dev, &desc)?;
        let mut s = match name {
            "mix" => {
                let (meshes, materials) = gallery_fill(&mut r)?;
                gallery_scene(&r, meshes, materials, [0.0; 3])?.0
            }
            "tex" => tex_gallery(&mut r)?,
            "gltf" => gltf_gallery(&mut r)?,
            _ => later_gallery_in(name, &mut r, format)?,
        };
        let mut off = Off::new(dev, GALLERY_W, GALLERY_H, format)?;
        off.submit(&mut r, &mut s)?;
        self.scenes.push(s);
        self.more.push(r);
        Ok(off)
    }

    fn step(&mut self) -> Result<()> {
        match self.step {
            0 => {
                match Instance::new() {
                    Ok(i) => self.inst = Some(i),
                    Err(e) => {
                        say(&format!("SKIP render3dtest: {e}"));
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
                        say(&format!("SKIP render3dtest: {e}"));
                        self.step = u32::MAX;
                        return Ok(());
                    }
                }
                let a = inst.adapter()?;
                let dev = a.create_device(&DeviceDesc::DEFAULT)?;
                let (i, l, f) = (a.info(), dev.limits(), dev.features());
                say(&format!("adapter: {} ({}), level {:?}, storage binding {} MiB, indirect-first-instance {}", i.name, i.backend, dev.level(), l.max_storage_binding >> 20, f.draw_indirect_first_instance));
                self.gallery(&dev)?;
                self.dev = Some(dev);
                self.step = 2;
            }
            2 => {
                let Some(dev) = self.dev.clone() else { return Ok(()) };
                dev.poll()?;
                while let Some(c) = self.counts.first() {
                    let mut w = alloc::vec![0u32; c.lists * 5];
                    match c.rb.read(0, &mut w) {
                        Err(GpuError::NotReady) => return Ok(()),
                        r => r?,
                    }
                    let total: u32 = (0..c.lists).map(|i| w[i * 5 + 1]).sum();
                    let what = format!("{}: the lists hold {total}, the exact test on the CPU says {} (+{} within a hair of a face)", c.name, c.want.0, c.want.1);
                    let good = total >= c.want.0 && total <= c.want.0 + c.want.1;
                    say(&format!("count {} {total}", c.name));
                    self.yes(good, &what);
                    self.counts.remove(0);
                }
                while let Some((name, off)) = self.keep.first() {
                    let img = match off.read() {
                        Err(GpuError::NotReady) => return Ok(()),
                        r => r?,
                    };
                    // long lines in pieces: consoles and the bridge have limits
                    let line: String = hex_line(name, &img);
                    for (i, piece) in line.as_bytes().chunks(16000).enumerate() {
                        let text = core::str::from_utf8(piece).unwrap_or("");
                        say(&format!("part {i} {text}"));
                    }
                    say("part end");
                    if name.starts_with("enc_") {
                        self.encoded.push((name, img));
                    }
                    self.keep.remove(0);
                }
                // (G5e2) the shader's sRGB against the float image encoded here, by the definition
                while let Some((f, tag, off)) = self.floats.first() {
                    let img = match off.read() {
                        Err(GpuError::NotReady) => return Ok(()),
                        r => r?,
                    };
                    let (f, tag) = (*f, *tag);
                    let enc = self.encoded.iter().find(|e| e.0 == tag).map(|e| e.1.clone()).unwrap_or_default();
                    let (mut n, mut m) = (0u32, 0u8);
                    for (p, c) in enc.chunks_exact(4).zip(img.chunks_exact(16)) {
                        let v = |k: usize| f32::from_le_bytes([c[k * 4], c[k * 4 + 1], c[k * 4 + 2], c[k * 4 + 3]]) as f64;
                        let d = (0..3).map(|k| p[k].abs_diff(srgb_encode(v(k)))).max().unwrap_or(0);
                        n += (d > 0) as u32;
                        m = m.max(d);
                    }
                    let fine = m <= 1 && enc.len() * 4 == img.len();
                    self.yes(fine, &format!("{tag}: the shader's sRGB against {f} (Rgba32Float) encoded on the CPU: {n} pixels differ, largest {m}"));
                    self.floats.remove(0);
                }
                dev.poll()?;
                self.yes(dev.poll().is_ok(), "the browser found no validation error in all of it");
                self.end();
            }
            _ => {}
        }
        Ok(())
    }
}

impl plat::web::WebApplication for Probe {
    fn frame_delay(&self) -> u32 {
        0
    }
    fn tick(&mut self, _time_ms: u32) {
        if self.step == u32::MAX {
            return;
        }
        if plat::time_us() > self.t0 + 60_000_000 {
            self.yes(false, "the browser's answers did not come in 60 s");
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

pub fn start() -> i32 {
    let t0 = plat::time_us();
    let p = Probe { step: 0, inst: None, dev: None, keep: Vec::new(), floats: Vec::new(), encoded: Vec::new(), scenes: Vec::new(), renderer: None, more: Vec::new(), counts: Vec::new(), ok: 0, bad: 0, t0 };
    if !plat::web::install(Box::new(p)) {
        return 2;
    }
    0
}
