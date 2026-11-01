// t_wgsl2 -- the parts of gputest's wgsl test beyond the numeric core (t_wgsl.rs calls them):
//
//   overrides    a shader with overrides of every kind (@id, by name, derived with float
//                arithmetic, without default, the workgroup size, a workgroup array sized by an
//                override-expression): Output::constants + Output::workgroup_size give the
//                pipeline constants (specialization constants), CompileOptions::constants
//                substitutes them (no specialization constant left); both = the CPU, defaults
//                too; WebGPU's pipeline errors (missing, unknown key, by name with @id, out of
//                range, workgroup size 0) are Err
//   robust       out-of-bounds indices of a runtime-sized array and of a fixed array in a
//                struct stay inside the binding (a guard after the bound range untouched);
//                without robustness the guard is overwritten (reported)
//   workgroup    workgroup memory is zeroed although a dispatch before left values there;
//                atomics in workgroup memory; workgroupUniformLoad
//   discard      a fragment shader discards the left half: the colour stays the clear colour
//                there, an atomic counter after discard counts only the right half, dpdx after
//                discard still works (demote at Voll, the flag and OpKill at Basis)
//   textures     textureStore out of bounds is skipped, textureLoad out of bounds is clamped
//                (storage textures and a sampled texture with mips), textureDimensions

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use gpu::*;

use crate::t_compute::GUARD;
use crate::{T, out, run_wait};

fn compile(t: &mut T, src: &str, opt: &wgsl::CompileOptions, what: &str) -> Option<wgsl::Output> {
    match wgsl::compile(src, opt) {
        Ok(o) => Some(o),
        Err(d) => {
            t.fail(&format!("{what}: compile: {}", d.first().map(|x| format!("{x}")).unwrap_or_default()));
            None
        }
    }
}

fn module(t: &mut T, dev: &Device, o: &wgsl::Output, entry: &str, what: &str) -> Option<ShaderModule> {
    let words = o.entry(entry)?.words.clone();
    t.res(ShaderModule::from_spirv(dev, &words), &format!("{what}: module"))
}

fn buffer(t: &mut T, dev: &Device, words: usize) -> Option<Buffer> {
    t.res(dev.create_buffer(&BufferDesc { size: words as u64 * 4, usage: BufferUsage::STORAGE, memory: MemoryKind::Device }), "buffer")
}

fn whole(b: &Buffer) -> Resource<'_> {
    Resource::Buffer { buffer: b, offset: 0, size: 0 }
}

/// one compute dispatch of entry `main` with one storage buffer at binding 0
fn dispatch1(t: &mut T, dev: &Device, m: &ShaderModule, consts: &[Constant], buf: Resource, groups: u32, what: &str) -> Option<()> {
    let bgl = t.res(dev.create_bind_group_layout(&[Binding { binding: 0, kind: BindingKind::StorageBuffer, stages: Stages::COMPUTE }]),
                    "layout")?;
    let pl = t.res(dev.create_pipeline_layout(&PipelineLayoutDesc { groups: &[&bgl], push_constants: 0 }), "pipeline layout")?;
    let pipe = t.res(dev.create_compute_pipeline(&ComputePipelineDesc { layout: &pl, shader: ShaderStage { module: m, entry: "main",
                                                                                                         constants: consts } }),
                     &format!("{what}: pipeline"))?;
    let bg = t.res(dev.create_bind_group(&BindGroupDesc { layout: &bgl, entries: &[BindEntry { binding: 0, resource: buf }] }), "bind group")?;
    let mut e = t.res(dev.encoder(), "encoder")?;
    e.set_compute_pipeline(&pipe);
    e.set_bind_group(0, &bg);
    e.dispatch(groups, 1, 1);
    e.barrier(Access::COMPUTE_WRITE, Access::COPY_READ);
    run_wait(t, dev, e.finish(), what)
}

const OVERRIDES: &str = "override wg: u32 = 64;
@id(5) override mul: i32 = 3;
override scale: f32 = 1.5;
override flip: bool = false;
override derived: f32 = scale * 2.0 + f32(mul);
override n: u32;
var<workgroup> tile: array<u32, wg * 2u>;
@group(0) @binding(0) var<storage, read_write> outp: array<u32>;
@compute @workgroup_size(wg)
fn main(@builtin(local_invocation_index) li: u32, @builtin(global_invocation_id) gid: vec3u) {
    tile[li * 2u] = li + 1u;
    workgroupBarrier();
    let i = gid.x;
    if i < n {
        let v = select(i32(i) * mul, -i32(i), flip);
        outp[i * 4u] = bitcast<u32>(v);
        outp[i * 4u + 1u] = bitcast<u32>(derived * f32(i));
        outp[i * 4u + 2u] = tile[((li + 1u) % wg) * 2u];
        outp[i * 4u + 3u] = wg;
    }
}";

/// the CPU result of OVERRIDES for (wg, mul, scale, flip, n)
fn overrides_ref(wg: u32, mul: i32, scale: f32, flip: bool, n: u32) -> Vec<u32> {
    let derived = scale * 2.0 + mul as f32;
    let mut v = Vec::new();
    for i in 0..n {
        let li = i % wg;
        v.push(if flip { (-(i as i32)) as u32 } else { (i as i32).wrapping_mul(mul) as u32 });
        v.push((derived * i as f32).to_bits());
        v.push((li + 1) % wg + 1);
        v.push(wg);
    }
    v
}

fn run_overrides(t: &mut T, dev: &Device, m: &ShaderModule, consts: &[Constant], wg: u32, n: u32, want: &[u32], what: &str) {
    let Some(b) = buffer(t, dev, n as usize * 4 + 4) else { return };
    let _ = dev.queue().write_buffer(&b, 0, &vec![GUARD; n as usize * 4 + 4]);
    if dispatch1(t, dev, m, consts, whole(&b), n.div_ceil(wg), what).is_none() {
        return;
    }
    let mut got = vec![0u32; n as usize * 4 + 4];
    if t.res(dev.queue().read_buffer(&b, 0, &mut got), "read").is_none() {
        return;
    }
    let bad = got[..want.len()].iter().zip(want).position(|(a, b)| a != b);
    t.ok(bad.is_none() && got[want.len()..].iter().all(|&x| x == GUARD),
         &format!("{what}: {n} results = the CPU{}", bad.map_or(String::new(), |k| format!(" (word {k}: {:#x}, CPU {:#x})", got[k], want[k]))));
}

pub fn overrides(t: &mut T, dev: &Device, minor: u32, lv: &str) {
    let opt = wgsl::CompileOptions::new(minor);
    let Some(o) = compile(t, OVERRIDES, &opt, "overrides") else { return };
    let e = o.entry("main");
    t.ok(e.is_some_and(|e| e.spec.len() == 7), &format!("{lv}: overrides: 7 specialization constants (the 5 overrides the body \
                                                         uses, the workgroup size and the array length hidden) ({:?})",
                                                        e.map(|e| e.spec.len())));
    t.ok(e.is_some_and(|e| e.workgroup_size == Some([64, 1, 1])), &format!("{lv}: overrides: the default workgroup size {:?}",
                                                                             e.map(|e| e.workgroup_size)));
    let Some(m) = module(t, dev, &o, "main", "overrides") else { return };
    // spec constants, every override given
    let given = [("wg", 32.0), ("5", 7.0), ("scale", 0.5), ("flip", 0.0), ("n", 1000.0)];
    match (o.constants("main", &given), o.workgroup_size("main", &given)) {
        (Ok(c), Ok(ws)) => {
            t.ok(ws == [32, 1, 1], &format!("{lv}: overrides: workgroup_size {ws:?}"));
            let consts: Vec<Constant> = c.iter().map(|&(id, value)| Constant { id, value }).collect();
            run_overrides(t, dev, &m, &consts, 32, 1000, &overrides_ref(32, 7, 0.5, false, 1000),
                          &format!("{lv}: overrides as specialization constants (wg 32, mul 7, derived 8)"));
        }
        (a, b) => t.fail(&format!("{lv}: overrides: constants {:?} / workgroup_size {:?}", a.err(), b.err())),
    }
    // the defaults (only n set), flip on
    let given = [("n", 700.0), ("flip", 1.0)];
    if let Some(c) = t.res(o.constants("main", &given).map_err(|_| GpuError::Invalid("constants")), "constants") {
        let consts: Vec<Constant> = c.iter().map(|&(id, value)| Constant { id, value }).collect();
        run_overrides(t, dev, &m, &consts, 64, 700, &overrides_ref(64, 3, 1.5, true, 700), &format!("{lv}: overrides with the defaults"));
    }
    // substituted: no specialization constants in the module
    let given = [("wg", 128.0), ("5", -2.0), ("scale", 4.0), ("flip", 0.0), ("n", 3000.0)];
    let so = wgsl::CompileOptions { constants: Some(&given), ..opt };
    if let Some(o2) = compile(t, OVERRIDES, &so, "overrides substituted") {
        let words = o2.entry("main").map(|e| e.words.clone()).unwrap_or_default();
        let none = shader::reflect::spec_constants(&words).map(|v| v.is_empty()).unwrap_or(false);
        t.ok(none, &format!("{lv}: overrides substituted: no specialization constant left"));
        if let Some(m2) = t.res(ShaderModule::from_spirv(dev, &words), "substituted module") {
            run_overrides(t, dev, &m2, &[], 128, 3000, &overrides_ref(128, -2, 4.0, false, 3000), &format!("{lv}: overrides substituted"));
        }
    }
    // WebGPU's pipeline errors
    let errs: [(&[(&str, f64)], &str); 5] = [
        (&[], "n without value"),
        (&[("n", 10.0), ("mul", 2.0)], "an @id override by name"),
        (&[("n", 10.0), ("nope", 2.0)], "an unknown key"),
        (&[("n", -1.0)], "u32 out of range"),
        (&[("n", 10.0), ("wg", 0.0)], "workgroup size 0"),
    ];
    for (g, what) in errs {
        let r = o.constants("main", g);
        t.ok(r.is_err(), &format!("{lv}: overrides: {what} is a pipeline error ({})", r.err().unwrap_or_default()));
    }
}

const ROBUST: &str = "struct S { a: array<u32, 8>, b: u32 }
@group(0) @binding(0) var<storage, read_write> data: array<u32>;
@group(0) @binding(1) var<storage, read_write> s: S;
@group(0) @binding(2) var<storage, read_write> res: array<u32, 64>;
@group(0) @binding(3) var<storage, read> src: array<u32>;
@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) gid: vec3u) {
    let i = gid.x;
    data[i] = i + 1u;
    s.a[i] = i + 1u;
    if i < 64u {
        res[i] = src[i + 1000000u];
    }
}";

pub fn robust(t: &mut T, dev: &Device, minor: u32, lv: &str) {
    for robust in [true, false] {
        let opt = wgsl::CompileOptions { robust, ..wgsl::CompileOptions::new(minor) };
        let Some(o) = compile(t, ROBUST, &opt, "robust") else { return };
        let Some(m) = module(t, dev, &o, "main", "robust") else { return };
        // data: words 0..64 bound, 64..1024 the guard; s: words 0..9 bound, then the guard
        let (Some(db), Some(sb), Some(rb), Some(src)) = (buffer(t, dev, 1024), buffer(t, dev, 1024), buffer(t, dev, 64), buffer(t, dev, 1024))
        else { return };
        let _ = dev.queue().write_buffer(&db, 0, &vec![GUARD; 1024]);
        let _ = dev.queue().write_buffer(&sb, 0, &vec![GUARD; 1024]);
        let _ = dev.queue().write_buffer(&src, 0, &(0..1024u32).map(|k| k * 10).collect::<Vec<u32>>());
        let b = |binding| Binding { binding, kind: BindingKind::StorageBuffer, stages: Stages::COMPUTE };
        let Some(bgl) = t.res(dev.create_bind_group_layout(&[b(0), b(1), b(2), Binding { binding: 3, kind: BindingKind::StorageBufferRead,
                                                                                        stages: Stages::COMPUTE }]), "layout") else { return };
        let Some(pl) = t.res(dev.create_pipeline_layout(&PipelineLayoutDesc { groups: &[&bgl], push_constants: 0 }), "layout") else { return };
        let Some(pipe) = t.res(dev.create_compute_pipeline(&ComputePipelineDesc { layout: &pl, shader: ShaderStage::new(&m, "main") }),
                               "pipeline") else { return };
        let Some(bg) = t.res(dev.create_bind_group(&BindGroupDesc { layout: &bgl, entries: &[
            BindEntry { binding: 0, resource: Resource::Buffer { buffer: &db, offset: 0, size: 256 } },
            BindEntry { binding: 1, resource: Resource::Buffer { buffer: &sb, offset: 0, size: 36 } },
            BindEntry { binding: 2, resource: whole(&rb) },
            BindEntry { binding: 3, resource: Resource::Buffer { buffer: &src, offset: 0, size: 64 } },
        ] }), "bind group") else { return };
        let Some(mut e) = t.res(dev.encoder(), "encoder") else { return };
        e.set_compute_pipeline(&pipe);
        e.set_bind_group(0, &bg);
        e.dispatch(4, 1, 1); // 256 invocations, 64 elements bound
        e.barrier(Access::COMPUTE_WRITE, Access::COPY_READ);
        if run_wait(t, dev, e.finish(), "robust").is_none() {
            return;
        }
        let (mut d, mut s, mut r) = (vec![0u32; 1024], vec![0u32; 1024], vec![0u32; 64]);
        let _ = dev.queue().read_buffer(&db, 0, &mut d);
        let _ = dev.queue().read_buffer(&sb, 0, &mut s);
        let _ = dev.queue().read_buffer(&rb, 0, &mut r);
        let guard_d = d[64..].iter().all(|&x| x == GUARD);
        let guard_s = s[9..].iter().all(|&x| x == GUARD) && s[8] == GUARD;
        if robust {
            t.ok(guard_d, &format!("{lv}: robust: writes past a runtime-sized array's binding (index up to 255 of 64) stay inside"));
            t.ok(d[..63].iter().enumerate().all(|(i, &x)| x == i as u32 + 1) && d[63] >= 64,
                 &format!("{lv}: robust: the elements in bounds as written, the last one from a clamped index ({})", d[63]));
            t.ok(guard_s && s[..7].iter().enumerate().all(|(i, &x)| x == i as u32 + 1),
                 &format!("{lv}: robust: a fixed array in a struct: clamped to its 8 elements, the member after it untouched"));
            t.ok(r.iter().all(|&x| x == 150), &format!("{lv}: robust: reads far out of bounds of a 16-element binding read its last \
                                                         element (150: {:?})", &r[..4]));
        } else {
            out(&format!("  info {lv}: without robustness: the guard after the runtime-sized array {}, after the struct's array {}",
                         if guard_d { "untouched" } else { "overwritten" }, if guard_s { "untouched" } else { "overwritten" }));
        }
    }
}

const WG_DIRTY: &str = "var<workgroup> junk: array<u32, 256>;
var<workgroup> junk2: atomic<u32>;
@group(0) @binding(0) var<storage, read_write> o: array<u32>;
@compute @workgroup_size(64)
fn main(@builtin(local_invocation_index) li: u32) {
    for (var k = 0u; k < 4u; k++) { junk[li * 4u + k] = 0xabcd0000u + li; }
    atomicStore(&junk2, 77u);
    workgroupBarrier();
    o[li] = junk[(li * 4u + 5u) % 256u] + atomicLoad(&junk2);
}";

const WG_TEST: &str = "var<workgroup> arr: array<u32, 256>;
var<workgroup> counter: atomic<u32>;
var<workgroup> flag: u32;
@group(0) @binding(0) var<storage, read_write> o: array<u32>;
@compute @workgroup_size(64)
fn main(@builtin(local_invocation_index) li: u32, @builtin(workgroup_id) wid: vec3u) {
    let z = arr[li * 4u] | arr[li * 4u + 3u];
    let c0 = atomicAdd(&counter, 1u);
    if li == 0u { flag = 5u + wid.x; }
    workgroupBarrier();
    let total = atomicLoad(&counter);
    let f = workgroupUniformLoad(&flag);
    o[(wid.x * 64u + li) * 3u] = z;
    o[(wid.x * 64u + li) * 3u + 1u] = total;
    o[(wid.x * 64u + li) * 3u + 2u] = f + u32(c0 >= 64u);
}";

pub fn workgroup(t: &mut T, dev: &Device, minor: u32, lv: &str) {
    let opt = wgsl::CompileOptions::new(minor);
    let (Some(od), Some(ot)) = (compile(t, WG_DIRTY, &opt, "workgroup dirty"), compile(t, WG_TEST, &opt, "workgroup")) else { return };
    let (Some(md), Some(mt)) = (module(t, dev, &od, "main", "dirty"), module(t, dev, &ot, "main", "workgroup")) else { return };
    let Some(b) = buffer(t, dev, 8 * 64 * 3) else { return };
    for _ in 0..3 {
        dispatch1(t, dev, &md, &[], whole(&b), 8, "dirty workgroup memory");
    }
    if dispatch1(t, dev, &mt, &[], whole(&b), 8, "workgroup").is_none() {
        return;
    }
    let mut got = vec![0u32; 8 * 64 * 3];
    let _ = dev.queue().read_buffer(&b, 0, &mut got);
    let zero = got.chunks(3).all(|c| c[0] == 0);
    let count = got.chunks(3).all(|c| c[1] == 64);
    let flag = got.chunks(3).enumerate().all(|(i, c)| c[2] == 5 + (i / 64) as u32);
    t.ok(zero, &format!("{lv}: workgroup: memory zeroed at the start (a dispatch before left values there)"));
    t.ok(count, &format!("{lv}: workgroup: an atomic counter in workgroup memory = 64 after the barrier"));
    t.ok(flag, &format!("{lv}: workgroup: workgroupUniformLoad reads what invocation 0 stored"));
}

const DISCARD: &str = "@group(0) @binding(0) var<storage, read_write> hits: array<atomic<u32>, 2>;
@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4f {
    let x = f32(i32(i & 1u) * 4 - 1);
    let y = f32(i32(i >> 1u) * 4 - 1);
    return vec4f(x, y, 0.0, 1.0);
}
@fragment fn fs(@builtin(position) p: vec4f) -> @location(0) vec4f {
    atomicAdd(&hits[1], 1u);
    if p.x < 32.0 { discard; }
    atomicAdd(&hits[0], 1u);
    let d = dpdx(p.x);
    return vec4f(1.0, d * 0.5, 0.0, 1.0);
}";

pub fn discard(t: &mut T, dev: &Device, minor: u32, lv: &str) {
    if !dev.features().fragment_stores {
        out(&format!("  SKIP {lv}: discard: no fragment stores"));
        return;
    }
    let opt = wgsl::CompileOptions::new(minor);
    let Some(o) = compile(t, DISCARD, &opt, "discard") else { return };
    let Some(m) = t.res(ShaderModule::from_spirv(dev, &o.entry("vs").map(|e| e.words.clone()).unwrap_or_default()), "vs") else { return };
    let Some(fm) = t.res(ShaderModule::from_spirv(dev, &o.entry("fs").map(|e| e.words.clone()).unwrap_or_default()), "fs") else { return };
    let Some(bgl) = t.res(dev.create_bind_group_layout(&[Binding { binding: 0, kind: BindingKind::StorageBuffer, stages: Stages::FRAGMENT }]),
                          "layout") else { return };
    let Some(pl) = t.res(dev.create_pipeline_layout(&PipelineLayoutDesc { groups: &[&bgl], push_constants: 0 }), "layout") else { return };
    let Some(pipe) = t.res(dev.create_render_pipeline(&RenderPipelineDesc {
        layout: &pl,
        vertex: ShaderStage::new(&m, "vs"),
        fragment: Some(ShaderStage::new(&fm, "fs")),
        buffers: &[],
        topology: Topology::Triangles,
        cull: Cull::None,
        front_cw: false,
        targets: &[ColorTarget { format: Format::Rgba8Unorm, blend: None, write_mask: 0xF }],
        depth: None,
    }), "discard pipeline") else { return };
    let Some(hb) = buffer(t, dev, 2) else { return };
    let _ = dev.queue().write_buffer(&hb, 0, &[0u32, 0]);
    let Some(bg) = t.res(dev.create_bind_group(&BindGroupDesc { layout: &bgl, entries: &[BindEntry { binding: 0, resource: whole(&hb) }] }),
                         "bind group") else { return };
    let Some(tex) = t.res(dev.create_texture(&TextureDesc::d2(64, 64, Format::Rgba8Unorm, TextureUsage::COLOR_TARGET | TextureUsage::COPY_SRC)),
                          "target") else { return };
    let Some(rb) = t.res(dev.create_buffer(&BufferDesc { size: 64 * 64 * 4, usage: BufferUsage::NONE, memory: MemoryKind::Readback }), "rb")
    else { return };
    let Some(mut e) = t.res(dev.encoder(), "encoder") else { return };
    e.begin_render(&RenderPassDesc { color: &[ColorAttachment { view: tex.view(), load: Load::Clear, clear: [0.0, 0.0, 1.0, 1.0], store: true,
                                                                before: Layout::Undefined, after: Layout::CopySrc }], depth: None });
    e.set_render_pipeline(&pipe);
    e.set_bind_group(0, &bg);
    e.draw(3, 1, 0, 0);
    e.end_render();
    e.copy_texture_to_buffer(TextureRegion::mip(&tex, 0), &rb, DataLayout::default());
    e.barrier(Access::COPY_WRITE | Access::GRAPHICS_WRITE, Access::HOST_READ | Access::COPY_READ);
    if run_wait(t, dev, e.finish(), "discard").is_none() {
        return;
    }
    let mut px = vec![0u8; 64 * 64 * 4];
    let _ = rb.read(0, &mut px);
    let mut hits = [0u32; 2];
    let _ = dev.queue().read_buffer(&hb, 0, &mut hits);
    let at = |x: usize, y: usize| [px[(y * 64 + x) * 4], px[(y * 64 + x) * 4 + 1], px[(y * 64 + x) * 4 + 2], px[(y * 64 + x) * 4 + 3]];
    let left = (0..64).all(|y| (0..32).all(|x| at(x, y) == [0, 0, 255, 255]));
    let right = (0..64).all(|y| (32..64).all(|x| at(x, y) == [255, 128, 0, 255] || at(x, y) == [255, 127, 0, 255]));
    t.ok(left, &format!("{lv}: discard: the left half keeps the clear colour"));
    t.ok(right, &format!("{lv}: discard: the right half is drawn, dpdx(p.x) = 1 after the discard ({:?})", at(40, 10)));
    t.ok(hits[0] == 32 * 64, &format!("{lv}: discard: the atomic after discard counted {} (the 2048 fragments kept)", hits[0]));
    t.ok(hits[1] >= 64 * 64, &format!("{lv}: discard: the atomic before discard counted {} (every fragment)", hits[1]));
}

const TEXTURES: &str = "@group(0) @binding(0) var st: texture_storage_2d<r32uint, write>;
@group(0) @binding(1) var<storage, read_write> o: array<u32>;
@group(0) @binding(2) var tx: texture_2d<f32>;
@group(0) @binding(3) var rt: texture_storage_2d<r32uint, read>;
@compute @workgroup_size(8, 8)
fn store_texels(@builtin(global_invocation_id) g: vec3u) {
    // 16 x 16 invocations over an 8 x 8 texture: the ones outside are skipped
    textureStore(st, vec2i(g.xy) - vec2i(4), vec4u(g.x * 100u + g.y, 0u, 0u, 1u));
}
@compute @workgroup_size(1)
fn load_texels() {
    let d = textureDimensions(tx, 1u);
    o[0] = d.x;
    o[1] = d.y;
    o[2] = textureNumLevels(tx);
    // out of bounds: clamped to the last texel / level
    let c = textureLoad(tx, vec2i(100, -3), 7);
    o[3] = bitcast<u32>(c.x);
    o[4] = textureLoad(rt, vec2i(99, 99)).x;
    o[5] = textureLoad(rt, vec2i(-1, 0)).x;
    o[6] = textureLoad(rt, vec2i(3, 5)).x;
}";

pub fn textures(t: &mut T, dev: &Device, minor: u32, lv: &str) {
    let opt = wgsl::CompileOptions::new(minor);
    let Some(o) = compile(t, TEXTURES, &opt, "textures") else { return };
    let (Some(mw), Some(mr)) = (module(t, dev, &o, "store_texels", "textures store"), module(t, dev, &o, "load_texels", "textures load")) else { return };
    let Some(st) = t.res(dev.create_texture(&TextureDesc::d2(8, 8, Format::R32Uint, TextureUsage::STORAGE | TextureUsage::COPY_SRC)), "st")
    else { return };
    let Some(tx) = t.res(dev.create_texture(&TextureDesc { mips: 3, ..TextureDesc::d2(8, 4, Format::R32Float, TextureUsage::SAMPLED
                                                                                       | TextureUsage::COPY_DST) }), "tx") else { return };
    // the mips: level 0 = 1.0, 1 = 2.0, 2 = 3.0
    for (mip, v) in [(0u32, 1.0f32), (1, 2.0), (2, 3.0)] {
        let n = ((8 >> mip) * (4 >> mip).max(1)) as usize;
        let data: Vec<u8> = (0..n).flat_map(|_| v.to_le_bytes()).collect();
        let _ = dev.queue().write_texture(&TextureRegion::mip(&tx, mip), &data, DataLayout::default(), Layout::Undefined, Layout::Sampled);
    }
    let Some(ob) = buffer(t, dev, 8) else { return };
    let bw = |binding, kind| Binding { binding, kind, stages: Stages::COMPUTE };
    let Some(lw) = t.res(dev.create_bind_group_layout(&[bw(0, BindingKind::StorageTexture)]), "layout") else { return };
    let Some(lr) = t.res(dev.create_bind_group_layout(&[bw(1, BindingKind::StorageBuffer), bw(2, BindingKind::Texture),
                                                        bw(3, BindingKind::StorageTexture)]), "layout") else { return };
    let (Some(plw), Some(plr)) = (t.res(dev.create_pipeline_layout(&PipelineLayoutDesc { groups: &[&lw], push_constants: 0 }), "pl"),
                                  t.res(dev.create_pipeline_layout(&PipelineLayoutDesc { groups: &[&lr], push_constants: 0 }), "pl"))
    else { return };
    let Some(pw) = t.res(dev.create_compute_pipeline(&ComputePipelineDesc { layout: &plw, shader: ShaderStage::new(&mw, "store_texels") }), "pw")
    else { return };
    let Some(pr) = t.res(dev.create_compute_pipeline(&ComputePipelineDesc { layout: &plr, shader: ShaderStage::new(&mr, "load_texels") }), "pr")
    else { return };
    let Some(gw) = t.res(dev.create_bind_group(&BindGroupDesc { layout: &lw, entries: &[BindEntry { binding: 0,
        resource: Resource::StorageTexture(st.view()) }] }), "bg") else { return };
    let Some(gr) = t.res(dev.create_bind_group(&BindGroupDesc { layout: &lr, entries: &[
        BindEntry { binding: 1, resource: whole(&ob) }, BindEntry { binding: 2, resource: Resource::Texture(tx.view()) },
        BindEntry { binding: 3, resource: Resource::StorageTexture(st.view()) }] }), "bg") else { return };
    let Some(rb) = t.res(dev.create_buffer(&BufferDesc { size: 256, usage: BufferUsage::NONE, memory: MemoryKind::Readback }), "rb")
    else { return };
    let Some(mut e) = t.res(dev.encoder(), "encoder") else { return };
    e.texture_barrier(&st, Layout::Undefined, Layout::General);
    e.set_compute_pipeline(&pw);
    e.set_bind_group(0, &gw);
    e.dispatch(2, 2, 1);
    e.barrier(Access::COMPUTE_WRITE, Access::COMPUTE_READ);
    e.set_compute_pipeline(&pr);
    e.set_bind_group(0, &gr);
    e.dispatch(1, 1, 1);
    e.barrier(Access::COMPUTE_WRITE, Access::COPY_READ);
    e.texture_barrier(&st, Layout::General, Layout::CopySrc);
    e.copy_texture_to_buffer(TextureRegion::mip(&st, 0), &rb, DataLayout::default());
    e.barrier(Access::COPY_WRITE, Access::HOST_READ);
    if run_wait(t, dev, e.finish(), "textures").is_none() {
        return;
    }
    let mut texels = vec![0u32; 64];
    let _ = rb.read(0, &mut texels);
    let mut r = vec![0u32; 8];
    let _ = dev.queue().read_buffer(&ob, 0, &mut r);
    // texel (x, y) was written by invocation (x + 4, y + 4)
    let stored = (0..8).all(|y| (0..8).all(|x| texels[y * 8 + x] == (x as u32 + 4) * 100 + y as u32 + 4));
    t.ok(stored, &format!("{lv}: textures: textureStore wrote the 64 texels, the 192 out-of-bounds stores skipped"));
    t.ok(r[0] == 4 && r[1] == 2 && r[2] == 3, &format!("{lv}: textures: textureDimensions(level 1) = {}x{}, 3 levels ({})", r[0], r[1], r[2]));
    t.ok(f32::from_bits(r[3]) == 3.0, &format!("{lv}: textures: textureLoad out of bounds (level 7) reads the last level ({})",
                                               f32::from_bits(r[3])));
    // a negative coordinate becomes a large u32 and is clamped to the last texel (any texel in
    // bounds is what WGSL allows)
    t.ok(r[4] == 1111 && r[5] == 1104 && r[6] == 709,
         &format!("{lv}: textures: storage textureLoad clamped: (99, 99) -> {} (1111), (-1, 0) -> {} (1104), (3, 5) -> {} (709)", r[4], r[5],
                  r[6]));
}
