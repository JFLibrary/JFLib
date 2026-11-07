// samples -- small shaders that exercise what the WGSL compiler will need, built with the
// module builder, with CPU references: specialization constants (WGSL override) and textures.
// apps/shadertest validates, disassembles and interprets them (spirv-val too when the Vulkan
// SDK is there), apps/gputest runs them on the GPU (docs/SHADER.md "Beispiel-Shader").
//
//   spec_fill(minor)        compute: pipeline constants for values, a derived constant per
//                           OpSpecConstantOp kind used, the workgroup size (SPEC_WG; Basis:
//                           the WorkgroupSize built-in, Voll: LocalSizeId) and a workgroup
//                           array of that size; SpecParams, spec_fill_ref
//   vs_fullscreen(minor)    vertex: one triangle over the whole target (no inputs)
//   fs_sample(minor)        fragment: texture + sampler, explicit LOD or implicit (mip selection)
//   fs_compare(minor)       fragment: depth texture + comparison sampler, explicit or implicit
//   cs_fetch(minor, k, arrayed, depth)   compute: OpImageFetch of every texel of a mip
//   cs_sample(minor, dim)   compute: OpImageSampleExplicitLod at coordinates from a buffer
//                           (2D, 2D array, cube, cube array)
//   cs_storage_write / cs_storage_read / cs_storage_rw(minor, fmt)   compute: storage textures
//   storage_texel(fmt, x, y)  the texel cs_storage_write writes (words as the format stores
//                           them after conversion: unorm bytes, f16 / f32 / int bits)
//   cs_query(minor)         compute: OpImageQuerySizeLod / OpImageQueryLevels / OpImageQuerySize
//   cs_bindless_sample()    compute (SPIR-V 1.6, Voll): textures and samplers indexed
//                           non-uniformly from a Bindless table
//
// Every module: entry point "main", set 0 (the Bindless sample: set 0 = the table, set 1 the
// output), push constants as each function's comment says (std430, offset 0).

use alloc::vec::Vec;

use crate::layout::Rule;
use crate::module::{Id, Module};
use crate::spv::{op, scope, sem, Bi, Cap, Dim, Fmt, Mode, Model, Sc};

// ---------------------------------------------------------------- specialization constants

/// the pipeline-constant ids of spec_fill
pub const SPEC_WG: u32 = 0;
pub const SPEC_MUL: u32 = 1;
pub const SPEC_ADD: u32 = 2;
pub const SPEC_SCALE: u32 = 3;
pub const SPEC_FLIP: u32 = 4;

/// the values of spec_fill's pipeline constants
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpecParams {
    /// workgroup size x (y = z = 1) and the length of the workgroup array
    pub wg: u32,
    pub mul: u32,
    pub add: i32,
    pub scale: f32,
    pub flip: bool,
}

impl SpecParams {
    /// the defaults in the module
    pub const DEFAULT: SpecParams = SpecParams { wg: 64, mul: 3, add: -5, scale: 0.5, flip: false };

    /// (SpecId, value bits) pairs of every constant (reflect::constant_values, the interpreter)
    pub fn bits(&self) -> Vec<(u32, u64)> {
        alloc::vec![(SPEC_WG, self.wg as u64), (SPEC_MUL, self.mul as u64), (SPEC_ADD, self.add as u32 as u64),
                    (SPEC_SCALE, self.scale.to_bits() as u64), (SPEC_FLIP, self.flip as u64)]
    }
}

/// what spec_fill writes for element i (4 words): w0 = the workgroup array's next slot
/// (written by the neighbour invocation: ((i / wg) * wg + (i % wg + 1) % wg) * mul, inverted
/// when flip), w1 = i as i32 + add, w2 = the bits of f32(i) * scale, w3 = flip | (32 < wg) << 1 |
/// (wg * 2) << 8 | wg << 20 (the derived constants)
pub fn spec_fill_ref(p: &SpecParams, n: u32) -> Vec<u32> {
    let mut out = Vec::with_capacity(n as usize * 4);
    for i in 0..n {
        let next = (i / p.wg) * p.wg + (i % p.wg + 1) % p.wg;
        let w0 = next.wrapping_mul(p.mul);
        out.push(if p.flip { !w0 } else { w0 });
        out.push((i as i32).wrapping_add(p.add) as u32);
        out.push(((i as f32) * p.scale).to_bits());
        out.push(p.flip as u32 | ((32 < p.wg) as u32) << 1 | (p.wg.wrapping_mul(2)) << 8 | p.wg << 20);
    }
    out
}

/// compute: set 0 binding 0 buffer { uint v[]; }, push constants { uint n; }; spec constants
/// SPEC_WG .. SPEC_FLIP (SpecParams::DEFAULT); invocation i < n writes spec_fill_ref's words
/// v[4i .. 4i + 4]
pub fn spec_fill(minor: u32) -> Vec<u32> {
    let mut m = Module::new(minor);
    let (void, u, i32t, f, b) = (m.ty_void(), m.ty_u32(), m.ty_i32(), m.ty_f32(), m.ty_bool());
    let uv3 = m.ty_vec(u, 3);
    let d = SpecParams::DEFAULT;
    let wg = m.spec_u32(SPEC_WG, d.wg);
    m.name(wg, "WG");
    let mul = m.spec_u32(SPEC_MUL, d.mul);
    m.name(mul, "MUL");
    let add = m.spec_i32(SPEC_ADD, d.add);
    m.name(add, "ADD");
    let scale = m.spec_f32(SPEC_SCALE, d.scale);
    m.name(scale, "SCALE");
    let flip = m.spec_bool(SPEC_FLIP, d.flip);
    m.name(flip, "FLIP");
    let (c0, c1, c2, c8, c20, c32) = (m.const_u32(0), m.const_u32(1), m.const_u32(2), m.const_u32(8), m.const_u32(20), m.const_u32(32));
    // the derived constants: one OpSpecConstantOp of each kind the shader needs
    let size = [wg, c1, c1];
    let wgvec = m.spec_composite(uv3, &size);
    let wgx = m.spec_op(u, op::CompositeExtract, &[wgvec], &[0]);
    let wg2 = m.spec_op(u, op::IMul, &[wg, c2], &[]);
    let wg2s = m.spec_op(u, op::ShiftLeftLogical, &[wg2, c8], &[]);
    let wgxs = m.spec_op(u, op::ShiftLeftLogical, &[wgx, c20], &[]);
    let big = m.spec_op(b, op::ULessThan, &[c32, wg], &[]);
    let noflip = m.spec_op(b, op::LogicalNot, &[flip], &[]);
    let flipbit = m.spec_op(u, op::Select, &[noflip, c0, c1], &[]);
    let derived = m.spec_op(u, op::BitwiseOr, &[wg2s, wgxs], &[]);
    let derived = m.spec_op(u, op::BitwiseOr, &[derived, flipbit], &[]);
    // resources
    let arr = m.ty_runtime_array(u, Some(4));
    let block = m.ty_struct_layout("Out", &[("v", arr)], Rule::Std430);
    let out = m.buffer(Sc::StorageBuffer, block, 0, 0);
    m.name(out, "out");
    let pblock = m.ty_struct_layout("Push", &[("n", u)], Rule::Std430);
    let pc = m.push_constant(pblock);
    m.name(pc, "pc");
    let tile_t = m.ty_array_id(u, wg);
    let tile = m.workgroup(tile_t);
    m.name(tile, "tile");
    let gid_v = m.builtin(Sc::Input, uv3, Bi::GlobalInvocationId);
    let lid_v = m.builtin(Sc::Input, uv3, Bi::LocalInvocationId);
    let (main, _) = m.begin_function(void, &[]);
    m.name(main, "main");
    let g = m.load(gid_v);
    let gid = m.extract(g, &[0]);
    let l = m.load(lid_v);
    let lid = m.extract(l, &[0]);
    // tile[lid] = gid * MUL; barrier; w0 = tile[(lid + 1) % WG]
    let slot = m.access(tile, &[lid]);
    let prod = m.imul(gid, mul);
    m.store(slot, prod);
    m.control_barrier(scope::Workgroup, scope::Workgroup, sem::AcquireRelease | sem::WorkgroupMemory);
    let pn = m.access_c(pc, &[0]);
    let n = m.load(pn);
    let inside = m.ult(gid, n);
    m.if_then(inside, |m| {
        let l1 = m.iadd(lid, c1);
        let nl = m.umod(l1, wg);
        let ns = m.access(tile, &[nl]);
        let w0 = m.load(ns);
        let w0v = m.local(u, None);
        m.store(w0v, w0);
        m.if_then(flip, |m| {
            let x = m.load(w0v);
            let nx = m.bnot(x);
            m.store(w0v, nx);
        });
        let w0 = m.load(w0v);
        let gi = m.bitcast(i32t, gid);
        let w1i = m.iadd(gi, add);
        let w1 = m.bitcast(u, w1i);
        let gf = m.u2f(f, gid);
        let w2f = m.fmul(gf, scale);
        let w2 = m.bitcast(u, w2f);
        let two = m.const_u32(2);
        let bigbit = m.select(big, two, c0);
        let w3 = m.bor(derived, bigbit);
        let four = m.const_u32(4);
        let base = m.imul(gid, four);
        for (k, w) in [w0, w1, w2, w3].into_iter().enumerate() {
            let kk = m.const_u32(k as u32);
            let at = m.iadd(base, kk);
            let p = m.access(out, &[c0, at]);
            m.store(p, w);
        }
    });
    m.ret();
    m.end_function();
    m.entry_point(Model::GLCompute, main, "main");
    m.workgroup_size(main, size);
    m.finish()
}

// ---------------------------------------------------------------- textures

/// the component type of a texture's texels in a shader
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Texel {
    Float,
    Uint,
    Sint,
}

/// the dimension of cs_sample's texture
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SampleDim {
    D2,
    D2Array,
    Cube,
    CubeArray,
}

fn sampled_type(m: &mut Module, k: Texel) -> Id {
    match k {
        Texel::Float => m.ty_f32(),
        Texel::Uint => m.ty_u32(),
        Texel::Sint => m.ty_i32(),
    }
}

fn out_buffer(m: &mut Module, set: u32, binding: u32, elem: Id, stride: u32) -> Id {
    let arr = m.ty_runtime_array(elem, Some(stride));
    let block = m.ty_struct_layout("Out", &[("v", arr)], Rule::Std430);
    let v = m.buffer(Sc::StorageBuffer, block, set, binding);
    m.name(v, "out");
    v
}

/// vertex: gl_Position = (-1, -1), (3, -1), (-1, 3) for vertices 0, 1, 2 (draw 3 vertices)
pub fn vs_fullscreen(minor: u32) -> Vec<u32> {
    let mut m = Module::new(minor);
    let (void, u, f) = (m.ty_void(), m.ty_u32(), m.ty_f32());
    let v4 = m.ty_vec(f, 4);
    let vi = m.builtin(Sc::Input, u, Bi::VertexIndex);
    let pos = m.builtin(Sc::Output, v4, Bi::Position);
    let (main, _) = m.begin_function(void, &[]);
    m.name(main, "main");
    let i = m.load(vi);
    let (c1, c2) = (m.const_u32(1), m.const_u32(2));
    let (lo, hi, z, one) = (m.const_f32(-1.0), m.const_f32(3.0), m.const_f32(0.0), m.const_f32(1.0));
    let is1 = m.ieq(i, c1);
    let is2 = m.ieq(i, c2);
    let x = m.select(is1, hi, lo);
    let y = m.select(is2, hi, lo);
    let p = m.construct(v4, &[x, y, z, one]);
    m.store(pos, p);
    m.ret();
    m.end_function();
    m.entry_point(Model::Vertex, main, "main");
    m.finish()
}

/// the fragment-shader push constants of fs_sample / fs_compare: uv = frag_coord.xy * scale
/// + bias; value: the LOD (fs_sample) or the depth reference (fs_compare); mode 0: explicit
/// LOD (0 for fs_compare), 1: implicit
pub const FS_PUSH_SIZE: u32 = 24;

fn fs_common(m: &mut Module) -> (Id, Id, Id) {
    let f = m.ty_f32();
    let u = m.ty_u32();
    let v2 = m.ty_vec(f, 2);
    let v4 = m.ty_vec(f, 4);
    let block = m.ty_struct_layout("Push", &[("scale", v2), ("bias", v2), ("value", f), ("mode", u)], Rule::Std430);
    let pc = m.push_constant(block);
    m.name(pc, "pc");
    let fc = m.builtin(Sc::Input, v4, Bi::FragCoord);
    let out = m.output(v4, 0);
    m.name(out, "color");
    (pc, fc, out)
}

fn fs_uv(m: &mut Module, pc: Id, fc: Id) -> Id {
    let c = m.load(fc);
    let xy = m.shuffle(c, c, &[0, 1]);
    let sp = m.access_c(pc, &[0]);
    let s = m.load(sp);
    let bp = m.access_c(pc, &[1]);
    let bias = m.load(bp);
    let t = m.fmul(xy, s);
    m.fadd(t, bias)
}

/// fragment: set 0 binding 0 texture_2d<f32>, binding 1 sampler; push constants (FS_PUSH_SIZE)
/// { vec2 scale; vec2 bias; float lod; uint mode; }; colour 0 = textureSampleLevel(t, s, uv,
/// lod) (mode 0) or textureSample(t, s, uv) (mode 1)
pub fn fs_sample(minor: u32) -> Vec<u32> {
    let mut m = Module::new(minor);
    let (void, f) = (m.ty_void(), m.ty_f32());
    let v4 = m.ty_vec(f, 4);
    let (pc, fc, out) = fs_common(&mut m);
    let img = m.ty_image(f, Dim::D2, 0, 0, 0, 1, Fmt::Unknown);
    let tex = m.resource(img, 0, 0);
    m.name(tex, "tex");
    let st = m.ty_sampler();
    let smp = m.resource(st, 0, 1);
    m.name(smp, "smp");
    let (main, _) = m.begin_function(void, &[]);
    m.name(main, "main");
    let res = m.local(v4, None);
    let uv = fs_uv(&mut m, pc, fc);
    let lp = m.access_c(pc, &[2]);
    let lod = m.load(lp);
    let mp = m.access_c(pc, &[3]);
    let mode = m.load(mp);
    let c0 = m.const_u32(0);
    let explicit = m.ieq(mode, c0);
    m.if_else(explicit, |m| {
        let t = m.load(tex);
        let s = m.load(smp);
        let si = m.sampled_image(t, s);
        let r = m.sample_lod(si, uv, lod);
        m.store(res, r);
    }, |m| {
        let t = m.load(tex);
        let s = m.load(smp);
        let si = m.sampled_image(t, s);
        let r = m.sample(si, uv);
        m.store(res, r);
    });
    let r = m.load(res);
    m.store(out, r);
    m.ret();
    m.end_function();
    m.entry_point(Model::Fragment, main, "main");
    m.exec_mode(main, Mode::OriginUpperLeft, &[]);
    m.finish()
}

/// fragment: set 0 binding 0 texture_depth_2d, binding 1 sampler_comparison; push constants as
/// fs_sample with value = the depth reference; colour 0 = (textureSampleCompareLevel(t, s, uv,
/// dref) (mode 0) or textureSampleCompare(t, s, uv, dref) (mode 1), 0, 0, 1)
pub fn fs_compare(minor: u32) -> Vec<u32> {
    let mut m = Module::new(minor);
    let (void, f) = (m.ty_void(), m.ty_f32());
    let (pc, fc, out) = fs_common(&mut m);
    let v4 = m.ty_vec(f, 4);
    let img = m.ty_image(f, Dim::D2, 1, 0, 0, 1, Fmt::Unknown);
    let tex = m.resource(img, 0, 0);
    m.name(tex, "depth");
    let st = m.ty_sampler();
    let smp = m.resource(st, 0, 1);
    m.name(smp, "cmp");
    let (main, _) = m.begin_function(void, &[]);
    m.name(main, "main");
    let res = m.local(f, None);
    let uv = fs_uv(&mut m, pc, fc);
    let dp = m.access_c(pc, &[2]);
    let dref = m.load(dp);
    let mp = m.access_c(pc, &[3]);
    let mode = m.load(mp);
    let c0 = m.const_u32(0);
    let explicit = m.ieq(mode, c0);
    let zero = m.const_f32(0.0);
    m.if_else(explicit, |m| {
        let t = m.load(tex);
        let s = m.load(smp);
        let si = m.sampled_image(t, s);
        let r = m.sample_dref_lod(si, uv, dref, zero);
        m.store(res, r);
    }, |m| {
        let t = m.load(tex);
        let s = m.load(smp);
        let si = m.sampled_image(t, s);
        let r = m.sample_dref(si, uv, dref);
        m.store(res, r);
    });
    let r = m.load(res);
    let one = m.const_f32(1.0);
    let c = m.construct(v4, &[r, zero, zero, one]);
    m.store(out, c);
    m.ret();
    m.end_function();
    m.entry_point(Model::Fragment, main, "main");
    m.exec_mode(main, Mode::OriginUpperLeft, &[]);
    m.finish()
}

/// the push constants of cs_fetch: { uint w, h, layers; int mip; uint base; } (20 bytes)
pub const FETCH_PUSH_SIZE: u32 = 20;

/// compute 8 x 8 x 1, one invocation per texel (x, y, layer) of mip `mip` (w x h x layers):
/// set 0 binding 0 a sampled 2D (array) texture of k (depth: texture_depth_2d), binding 1
/// buffer { uint v[]; }; v[base + 4 * ((layer * h + y) * w + x) + c] = the bits of component c
/// of textureLoad(t, (x, y[, layer]), mip)
pub fn cs_fetch(minor: u32, k: Texel, arrayed: bool, depth: bool) -> Vec<u32> {
    let mut m = Module::new(minor);
    let (void, u, i) = (m.ty_void(), m.ty_u32(), m.ty_i32());
    let uv3 = m.ty_vec(u, 3);
    let st = sampled_type(&mut m, k);
    let img = m.ty_image(st, Dim::D2, depth as u32, arrayed as u32, 0, 1, Fmt::Unknown);
    let tex = m.resource(img, 0, 0);
    m.name(tex, "tex");
    let out = out_buffer(&mut m, 0, 1, u, 4);
    let block = m.ty_struct_layout("Push", &[("w", u), ("h", u), ("layers", u), ("mip", i), ("base", u)], Rule::Std430);
    let pc = m.push_constant(block);
    m.name(pc, "pc");
    let gid_v = m.builtin(Sc::Input, uv3, Bi::GlobalInvocationId);
    let (main, _) = m.begin_function(void, &[]);
    m.name(main, "main");
    let g = m.load(gid_v);
    let (x, y, z) = (m.extract(g, &[0]), m.extract(g, &[1]), m.extract(g, &[2]));
    let field = |m: &mut Module, k: u32| {
        let p = m.access_c(pc, &[k]);
        m.load(p)
    };
    let (w, h, layers) = (field(&mut m, 0), field(&mut m, 1), field(&mut m, 2));
    let ix = m.ult(x, w);
    let iy = m.ult(y, h);
    let iz = m.ult(z, layers);
    let a = m.land(ix, iy);
    let inside = m.land(a, iz);
    m.if_then(inside, |m| {
        let mip = field(m, 3);
        let base = field(m, 4);
        let (xi, yi, zi) = (m.bitcast(i, x), m.bitcast(i, y), m.bitcast(i, z));
        let coord = if arrayed {
            let iv3 = m.ty_vec(i, 3);
            m.construct(iv3, &[xi, yi, zi])
        } else {
            let iv2 = m.ty_vec(i, 2);
            m.construct(iv2, &[xi, yi])
        };
        let t = m.load(tex);
        let texel = m.fetch(t, coord, mip);
        let zh = m.imul(z, h);
        let r = m.iadd(zh, y);
        let rw = m.imul(r, w);
        let idx = m.iadd(rw, x);
        let four = m.const_u32(4);
        let i4 = m.imul(idx, four);
        let at0 = m.iadd(base, i4);
        let c0 = m.const_u32(0);
        for c in 0..4u32 {
            let comp = m.extract(texel, &[c]);
            let bits = if k == Texel::Uint { comp } else { m.bitcast(u, comp) };
            let kc = m.const_u32(c);
            let at = m.iadd(at0, kc);
            let p = m.access(out, &[c0, at]);
            m.store(p, bits);
        }
    });
    m.ret();
    m.end_function();
    m.entry_point(Model::GLCompute, main, "main");
    m.exec_mode(main, Mode::LocalSize, &[8, 8, 1]);
    m.finish()
}

/// compute 64 x 1 x 1: set 0 binding 0 a float texture of dim, binding 1 sampler, binding 2
/// readonly buffer { vec4 at[]; } (two per sample: the coordinate (xy: 2D, xyz: 2D array with
/// the layer in z / cube direction, xyzw: cube array with the cube in w) and (lod, 0, 0, 0)),
/// binding 3 buffer { vec4 v[]; }; push constants { uint count; }:
/// v[i] = textureSampleLevel(t, s, coordinate i, lod i)
pub fn cs_sample(minor: u32, dim: SampleDim) -> Vec<u32> {
    let mut m = Module::new(minor);
    let (void, u, f) = (m.ty_void(), m.ty_u32(), m.ty_f32());
    let uv3 = m.ty_vec(u, 3);
    let v4 = m.ty_vec(f, 4);
    let (d, arrayed) = match dim {
        SampleDim::D2 => (Dim::D2, 0),
        SampleDim::D2Array => (Dim::D2, 1),
        SampleDim::Cube => (Dim::Cube, 0),
        SampleDim::CubeArray => (Dim::Cube, 1),
    };
    let img = m.ty_image(f, d, 0, arrayed, 0, 1, Fmt::Unknown);
    let tex = m.resource(img, 0, 0);
    m.name(tex, "tex");
    let sty = m.ty_sampler();
    let smp = m.resource(sty, 0, 1);
    m.name(smp, "smp");
    let arr = m.ty_runtime_array(v4, Some(16));
    let iblock = m.ty_struct_layout("At", &[("at", arr)], Rule::Std430);
    m.member_decorate(iblock, 0, crate::spv::Dec::NonWritable, &[]);
    let at = m.buffer(Sc::StorageBuffer, iblock, 0, 2);
    m.name(at, "at");
    let out = out_buffer(&mut m, 0, 3, v4, 16);
    let block = m.ty_struct_layout("Push", &[("count", u)], Rule::Std430);
    let pc = m.push_constant(block);
    m.name(pc, "pc");
    let gid_v = m.builtin(Sc::Input, uv3, Bi::GlobalInvocationId);
    let (main, _) = m.begin_function(void, &[]);
    m.name(main, "main");
    let g = m.load(gid_v);
    let idx = m.extract(g, &[0]);
    let cp = m.access_c(pc, &[0]);
    let count = m.load(cp);
    let inside = m.ult(idx, count);
    m.if_then(inside, |m| {
        let c0 = m.const_u32(0);
        let two = m.const_u32(2);
        let one = m.const_u32(1);
        let k = m.imul(idx, two);
        let pa = m.access(at, &[c0, k]);
        let c = m.load(pa);
        let k1 = m.iadd(k, one);
        let pl = m.access(at, &[c0, k1]);
        let lv = m.load(pl);
        let lod = m.extract(lv, &[0]);
        let coord = match dim {
            SampleDim::D2 => m.shuffle(c, c, &[0, 1]),
            SampleDim::D2Array | SampleDim::Cube => m.shuffle(c, c, &[0, 1, 2]),
            SampleDim::CubeArray => c,
        };
        let t = m.load(tex);
        let s = m.load(smp);
        let si = m.sampled_image(t, s);
        let r = m.sample_lod(si, coord, lod);
        let po = m.access(out, &[c0, idx]);
        m.store(po, r);
    });
    m.ret();
    m.end_function();
    m.entry_point(Model::GLCompute, main, "main");
    m.exec_mode(main, Mode::LocalSize, &[64, 1, 1]);
    m.finish()
}

/// the texel kind of a storage format
pub fn storage_texel_kind(fmt: Fmt) -> Texel {
    match fmt {
        Fmt::Rgba32ui | Fmt::Rgba16ui | Fmt::Rgba8ui | Fmt::R32ui | Fmt::Rg32ui => Texel::Uint,
        Fmt::Rgba32i | Fmt::Rgba16i | Fmt::Rgba8i | Fmt::R32i | Fmt::Rg32i => Texel::Sint,
        _ => Texel::Float,
    }
}

/// the integer pattern of storage texel (x, y), x, y < 16
fn pattern(x: u32, y: u32) -> [u32; 4] {
    let p0 = x * 16 + y;
    [p0, 255 - p0, (x * 7 + y * 3) & 255, 128 + x]
}

/// (scale, offset) of the float storage pattern: value = p * scale + offset, exact in the format
/// (rgba8unorm: p / 255, rounded by the format to byte p)
fn float_map(fmt: Fmt) -> (f32, f32) {
    match fmt {
        Fmt::Rgba8 => (1.0 / 255.0, 0.0),
        Fmt::Rgba16f | Fmt::R16f | Fmt::Rg16f => (0.25, 0.0),
        _ => (0.5, 0.25),
    }
}

/// what cs_storage_write puts at (x, y) of a texture of fmt, as the texture stores it: for
/// Rgba8 the four unorm bytes in one word (r in the low byte), for Rgba16f two words of two
/// halves (r low), for 32-bit formats one word per channel; channels the format lacks dropped
pub fn storage_texel(fmt: Fmt, x: u32, y: u32) -> Vec<u32> {
    let p = pattern(x, y);
    let (s, o) = float_map(fmt);
    let fl = |v: u32| (v as f32) * s + o;
    match fmt {
        Fmt::Rgba8 => alloc::vec![p[0] | p[1] << 8 | p[2] << 16 | p[3] << 24],
        Fmt::Rgba16f => {
            let h = |v: u32| crate::reflect::f16_from_f32(fl(v)) as u32;
            alloc::vec![h(p[0]) | h(p[1]) << 16, h(p[2]) | h(p[3]) << 16]
        }
        Fmt::R32f => alloc::vec![fl(p[0]).to_bits()],
        Fmt::Rgba32f => p.iter().map(|&v| fl(v).to_bits()).collect(),
        Fmt::R32ui => alloc::vec![p[0] | 0xABC0_0000],
        Fmt::Rgba32ui => p.iter().map(|&v| v | 0xABC0_0000).collect(),
        Fmt::R32i => alloc::vec![(-(p[0] as i32) - 1000) as u32],
        Fmt::Rgba32i => p.iter().map(|&v| (-(v as i32) - 1000) as u32).collect(),
        _ => Vec::new(),
    }
}

/// what textureLoad gives for storage_texel (four words: float bits, uint or int) -- the
/// format's missing channels as 0, 0, 1 (float) / 0, 0, 1 (integer)
pub fn storage_read(fmt: Fmt, x: u32, y: u32) -> [u32; 4] {
    let p = pattern(x, y);
    let (s, o) = float_map(fmt);
    let fl = |v: u32| (v as f32) * s + o;
    let one_f = 1.0f32.to_bits();
    match storage_texel_kind(fmt) {
        Texel::Float => {
            let v = |c: usize| match fmt {
                Fmt::Rgba8 => (p[c] as f32 / 255.0).to_bits(),
                _ => fl(p[c]).to_bits(),
            };
            if fmt == Fmt::R32f { [v(0), 0, 0, one_f] } else { [v(0), v(1), v(2), v(3)] }
        }
        Texel::Uint => {
            let v = |c: usize| p[c] | 0xABC0_0000;
            if fmt == Fmt::R32ui { [v(0), 0, 0, 1] } else { [v(0), v(1), v(2), v(3)] }
        }
        Texel::Sint => {
            let v = |c: usize| (-(p[c] as i32) - 1000) as u32;
            if fmt == Fmt::R32i { [v(0), 0, 0, 1] } else { [v(0), v(1), v(2), v(3)] }
        }
    }
}

fn storage_image(m: &mut Module, fmt: Fmt) -> (Id, Id) {
    let st = sampled_type(m, storage_texel_kind(fmt));
    let img = m.ty_image(st, Dim::D2, 0, 0, 0, 2, fmt);
    let v = m.resource(img, 0, 0);
    m.name(v, "img");
    (img, v)
}

fn storage_xy(m: &mut Module, pc: Id) -> (Id, Id, Id, Id, Id) {
    let u = m.ty_u32();
    let uv3 = m.ty_vec(u, 3);
    let gid_v = m.builtin(Sc::Input, uv3, Bi::GlobalInvocationId);
    let g = m.load(gid_v);
    let (x, y) = (m.extract(g, &[0]), m.extract(g, &[1]));
    let wp = m.access_c(pc, &[0]);
    let w = m.load(wp);
    let hp = m.access_c(pc, &[1]);
    let h = m.load(hp);
    let ix = m.ult(x, w);
    let iy = m.ult(y, h);
    let inside = m.land(ix, iy);
    (x, y, w, h, inside)
}

fn storage_push(m: &mut Module) -> Id {
    let u = m.ty_u32();
    let block = m.ty_struct_layout("Push", &[("w", u), ("h", u)], Rule::Std430);
    let pc = m.push_constant(block);
    m.name(pc, "pc");
    pc
}

fn ivec2_of(m: &mut Module, x: Id, y: Id) -> Id {
    let i = m.ty_i32();
    let iv2 = m.ty_vec(i, 2);
    let (xi, yi) = (m.bitcast(i, x), m.bitcast(i, y));
    m.construct(iv2, &[xi, yi])
}

fn compute_main(m: &mut Module) -> Id {
    let void = m.ty_void();
    let (main, _) = m.begin_function(void, &[]);
    m.name(main, "main");
    main
}

fn compute_end(m: &mut Module, main: Id, wg: [u32; 3]) -> Vec<u32> {
    m.ret();
    m.end_function();
    m.entry_point(Model::GLCompute, main, "main");
    m.exec_mode(main, Mode::LocalSize, &wg);
    m.finish()
}

/// compute 8 x 8 x 1: set 0 binding 0 a write-only storage texture (texture_storage_2d<fmt,
/// write>), push constants { uint w, h; }: textureStore of storage_texel's pattern at every
/// (x, y) < (w, h) (w, h <= 16)
pub fn cs_storage_write(minor: u32, fmt: Fmt) -> Vec<u32> {
    let mut m = Module::new(minor);
    let (f, i) = (m.ty_f32(), m.ty_i32());
    let (_, img) = storage_image(&mut m, fmt);
    m.decorate(img, crate::spv::Dec::NonReadable, &[]);
    let pc = storage_push(&mut m);
    let main = compute_main(&mut m);
    let (x, y, _, _, inside) = storage_xy(&mut m, pc);
    m.if_then(inside, |m| {
        // p = (x * 16 + y, 255 - p0, (x * 7 + y * 3) & 255, 128 + x)
        let c = |m: &mut Module, v: u32| m.const_u32(v);
        let (c16, c255, c7, c3, c128) = (c(m, 16), c(m, 255), c(m, 7), c(m, 3), c(m, 128));
        let x16 = m.imul(x, c16);
        let p0 = m.iadd(x16, y);
        let p1 = m.isub(c255, p0);
        let x7 = m.imul(x, c7);
        let y3 = m.imul(y, c3);
        let s = m.iadd(x7, y3);
        let p2 = m.band(s, c255);
        let p3 = m.iadd(c128, x);
        let ps = [p0, p1, p2, p3];
        let comps: Vec<Id> = match storage_texel_kind(fmt) {
            Texel::Float => {
                let (sc, of) = float_map(fmt);
                let (scale, off) = (m.const_f32(sc), m.const_f32(of));
                ps.iter().map(|&p| {
                    let pf = m.u2f(f, p);
                    let t = m.fmul(pf, scale);
                    m.fadd(t, off)
                }).collect()
            }
            Texel::Uint => {
                let hi = m.const_u32(0xABC0_0000);
                ps.iter().map(|&p| m.bor(p, hi)).collect()
            }
            Texel::Sint => {
                let k = m.const_i32(1000);
                ps.iter().map(|&p| {
                    let pi = m.bitcast(i, p);
                    let n = m.ineg(pi);
                    m.isub(n, k)
                }).collect()
            }
        };
        let st = m.type_of(comps[0]);
        let v4 = m.ty_vec(st, 4);
        let texel = m.construct(v4, &comps);
        let coord = ivec2_of(m, x, y);
        let im = m.load(img);
        m.image_write(im, coord, texel);
    });
    compute_end(&mut m, main, [8, 8, 1])
}

/// compute 8 x 8 x 1: set 0 binding 0 a read-only storage texture of fmt, binding 1 buffer
/// { uint v[]; }, push constants { uint w, h; }: v[4 (y w + x) + c] = the bits of component c
/// of textureLoad(img, (x, y)); invocation (0, 0) also writes textureDimensions(img) (OpImageQuerySize)
/// to v[4 w h], v[4 w h + 1]
pub fn cs_storage_read(minor: u32, fmt: Fmt) -> Vec<u32> {
    let mut m = Module::new(minor);
    let (u, i) = (m.ty_u32(), m.ty_i32());
    let (_, img) = storage_image(&mut m, fmt);
    m.decorate(img, crate::spv::Dec::NonWritable, &[]);
    let out = out_buffer(&mut m, 0, 1, u, 4);
    let pc = storage_push(&mut m);
    let main = compute_main(&mut m);
    let (x, y, w, h, inside) = storage_xy(&mut m, pc);
    let kind = storage_texel_kind(fmt);
    m.if_then(inside, |m| {
        let coord = ivec2_of(m, x, y);
        let im = m.load(img);
        let texel = m.image_read(im, coord);
        let yw = m.imul(y, w);
        let idx = m.iadd(yw, x);
        let four = m.const_u32(4);
        let base = m.imul(idx, four);
        let c0 = m.const_u32(0);
        for c in 0..4u32 {
            let comp = m.extract(texel, &[c]);
            let bits = if kind == Texel::Uint { comp } else { m.bitcast(u, comp) };
            let kc = m.const_u32(c);
            let at = m.iadd(base, kc);
            let p = m.access(out, &[c0, at]);
            m.store(p, bits);
        }
    });
    let c0 = m.const_u32(0);
    let x0 = m.ieq(x, c0);
    let y0 = m.ieq(y, c0);
    let first = m.land(x0, y0);
    m.if_then(first, |m| {
        let iv2 = m.ty_vec(i, 2);
        let im = m.load(img);
        let size = m.image_size(iv2, im);
        let wh = m.imul(w, h);
        let four = m.const_u32(4);
        let at = m.imul(wh, four);
        for k in 0..2u32 {
            let s = m.extract(size, &[k]);
            let su = m.bitcast(u, s);
            let kk = m.const_u32(k);
            let a = m.iadd(at, kk);
            let p = m.access(out, &[c0, a]);
            m.store(p, su);
        }
    });
    compute_end(&mut m, main, [8, 8, 1])
}

/// what cs_storage_rw makes of a texel value v (one channel: r32float, r32uint, r32sint)
pub fn storage_rw_ref(fmt: Fmt, v: u32) -> u32 {
    match storage_texel_kind(fmt) {
        Texel::Float => (f32::from_bits(v) * 2.0 + 1.0).to_bits(),
        Texel::Uint => v.wrapping_mul(3).wrapping_add(1),
        Texel::Sint => (v as i32).wrapping_sub(5) as u32,
    }
}

/// compute 8 x 8 x 1: set 0 binding 0 a read-write storage texture of a one-channel 32-bit
/// fmt, push constants { uint w, h; }: every texel v becomes storage_rw_ref(v) (v * 2 + 1,
/// v * 3 + 1, v - 5)
pub fn cs_storage_rw(minor: u32, fmt: Fmt) -> Vec<u32> {
    let mut m = Module::new(minor);
    let (_, img) = storage_image(&mut m, fmt);
    let pc = storage_push(&mut m);
    let main = compute_main(&mut m);
    let (x, y, _, _, inside) = storage_xy(&mut m, pc);
    let kind = storage_texel_kind(fmt);
    m.if_then(inside, |m| {
        let coord = ivec2_of(m, x, y);
        let im = m.load(img);
        let texel = m.image_read(im, coord);
        let v = m.extract(texel, &[0]);
        let r = match kind {
            Texel::Float => {
                let (two, one) = (m.const_f32(2.0), m.const_f32(1.0));
                let t = m.fmul(v, two);
                m.fadd(t, one)
            }
            Texel::Uint => {
                let (three, one) = (m.const_u32(3), m.const_u32(1));
                let t = m.imul(v, three);
                m.iadd(t, one)
            }
            Texel::Sint => {
                let five = m.const_i32(5);
                m.isub(v, five)
            }
        };
        let st = m.type_of(r);
        let v4 = m.ty_vec(st, 4);
        let texel = m.construct(v4, &[r, r, r, r]);
        let im = m.load(img);
        m.image_write(im, coord, texel);
    });
    compute_end(&mut m, main, [8, 8, 1])
}

/// the bindings of cs_query (set 0): 0 texture_2d<f32>, 1 texture_2d_array<f32>, 2
/// texture_cube<f32>, 3 texture_cube_array<f32>, 4 texture_depth_2d, 5 texture_2d<f32> (meant
/// for a view of some mips), 6 texture_storage_2d<r32float, read>; binding 7 the output
pub const QUERY_SAMPLED: u32 = 6;

/// compute 1 x 1 x 1: for each sampled binding b < QUERY_SAMPLED: v[12 b .. 12 b + 4] =
/// textureDimensions(t, 0) (with the layers of arrays), v[12 b + 4 ..] = at mip 1, v[12 b + 8] =
/// textureNumLevels(t) (the rest of the 12 words 0); v[72 ..] = textureDimensions of the
/// storage texture (OpImageQuerySize); every query as uint components (0 where the result has
/// fewer)
pub fn cs_query(minor: u32) -> Vec<u32> {
    let mut m = Module::new(minor);
    let (u, f) = (m.ty_u32(), m.ty_f32());
    let kinds: [(Dim, u32, u32); 6] = [(Dim::D2, 0, 0), (Dim::D2, 0, 1), (Dim::Cube, 0, 0), (Dim::Cube, 0, 1), (Dim::D2, 1, 0),
                                        (Dim::D2, 0, 0)];
    let mut texs = Vec::new();
    for (b, &(d, depth, arrayed)) in kinds.iter().enumerate() {
        let img = m.ty_image(f, d, depth, arrayed, 0, 1, Fmt::Unknown);
        let v = m.resource(img, 0, b as u32);
        texs.push((v, arrayed));
    }
    let simg = m.ty_image(f, Dim::D2, 0, 0, 0, 2, Fmt::R32f);
    let sv = m.resource(simg, 0, QUERY_SAMPLED);
    m.decorate(sv, crate::spv::Dec::NonWritable, &[]);
    let out = out_buffer(&mut m, 0, QUERY_SAMPLED + 1, u, 4);
    let main = compute_main(&mut m);
    let c0 = m.const_u32(0);
    let store_vec = |m: &mut Module, base: u32, v: Id, n: u32| {
        for k in 0..n {
            let c = if n == 1 { v } else { m.extract(v, &[k]) };
            let at = m.const_u32(base + k);
            let p = m.access(out, &[c0, at]);
            m.store(p, c);
        }
    };
    for (b, &(v, arrayed)) in texs.iter().enumerate() {
        let n = 2 + arrayed; // 2D and Cube: width, height (+ layers)
        let rt = m.ty_vec(u, n);
        for (slot, lod) in [(0u32, 0i32), (4, 1)] {
            let t = m.load(v);
            let l = m.const_i32(lod);
            let s = m.image_size_lod(rt, t, l);
            store_vec(&mut m, 12 * b as u32 + slot, s, n);
        }
        let t = m.load(v);
        let lv = m.image_levels(u, t);
        store_vec(&mut m, 12 * b as u32 + 8, lv, 1);
    }
    let uv2 = m.ty_vec(u, 2);
    let t = m.load(sv);
    let s = m.image_size(uv2, t);
    store_vec(&mut m, 12 * QUERY_SAMPLED, s, 2);
    compute_end(&mut m, main, [1, 1, 1])
}

/// compute 64 x 1 x 1, SPIR-V 1.6 (level Voll): set 0 the Bindless table (binding 0
/// texture_2d<f32> texs[], binding 1 sampler smps[]), set 1 binding 0 buffer { vec4 v[]; }, push
/// constants { uint count; uint ntex; uint nsmp; float u; float v; }: invocation i < count
/// samples texs[i % ntex] with smps[i % nsmp] at (u, v), LOD 0 -- indices that differ between
/// invocations, so the resources are decorated NonUniform
pub fn cs_bindless_sample() -> Vec<u32> {
    let mut m = Module::new(6);
    let (u, f) = (m.ty_u32(), m.ty_f32());
    let v2 = m.ty_vec(f, 2);
    let v4 = m.ty_vec(f, 4);
    let uv3 = m.ty_vec(u, 3);
    let img = m.ty_image(f, Dim::D2, 0, 0, 0, 1, Fmt::Unknown);
    let imgs = m.ty_runtime_array(img, None);
    let texs = m.resource(imgs, 0, 0);
    m.name(texs, "texs");
    let st = m.ty_sampler();
    let sts = m.ty_runtime_array(st, None);
    let smps = m.resource(sts, 0, 1);
    m.name(smps, "smps");
    m.capability(Cap::SampledImageArrayDynamicIndexing);
    let out = out_buffer(&mut m, 1, 0, v4, 16);
    let block = m.ty_struct_layout("Push", &[("count", u), ("ntex", u), ("nsmp", u), ("u", f), ("v", f)], Rule::Std430);
    let pc = m.push_constant(block);
    m.name(pc, "pc");
    let gid_v = m.builtin(Sc::Input, uv3, Bi::GlobalInvocationId);
    let main = compute_main(&mut m);
    let g = m.load(gid_v);
    let idx = m.extract(g, &[0]);
    let field = |m: &mut Module, k: u32| {
        let p = m.access_c(pc, &[k]);
        m.load(p)
    };
    let count = field(&mut m, 0);
    let inside = m.ult(idx, count);
    m.if_then(inside, |m| {
        let ntex = field(m, 1);
        let nsmp = field(m, 2);
        let (cu, cv) = (field(m, 3), field(m, 4));
        let ti = m.umod(idx, ntex);
        let si = m.umod(idx, nsmp);
        m.nonuniform(ti);
        m.nonuniform(si);
        let tp = m.access(texs, &[ti]);
        m.nonuniform(tp);
        let t = m.load(tp);
        m.nonuniform(t);
        let sp = m.access(smps, &[si]);
        m.nonuniform(sp);
        let s = m.load(sp);
        m.nonuniform(s);
        let sim = m.sampled_image(t, s);
        m.nonuniform(sim);
        let uv = m.construct(v2, &[cu, cv]);
        let zero = m.const_f32(0.0);
        let r = m.sample_lod(sim, uv, zero);
        let c0 = m.const_u32(0);
        let po = m.access(out, &[c0, idx]);
        m.store(po, r);
    });
    compute_end(&mut m, main, [64, 1, 1])
}
