// basics -- golden bytes of a tiny module, the builder, the disassembler (golden files of the
// benchmark shaders), the layout helper, determinism, and --write; samples() lists the
// modules of shader::samples (specialization constants, textures) the other groups check

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use shader::bench::{self, Shader};
use shader::{dis, layout, op, parse, Bi, Cap, Dim, Fmt, Id, Mode, Model, Module, Rule, Sc};

use crate::T;

/// every module of shader::samples by name (both levels where they differ)
pub fn samples() -> Vec<(String, Vec<u32>)> {
    use shader::samples::{self as s, SampleDim, Texel};
    let mut v: Vec<(String, Vec<u32>)> = Vec::new();
    for minor in [3u32, 6] {
        let sfx = if minor == 3 { "" } else { "_16" };
        v.push((format!("spec_fill{}", sfx), s::spec_fill(minor)));
    }
    v.push((String::from("vs_fullscreen"), s::vs_fullscreen(3)));
    v.push((String::from("fs_sample"), s::fs_sample(3)));
    v.push((String::from("fs_compare"), s::fs_compare(3)));
    for (k, kn) in [(Texel::Float, "f"), (Texel::Uint, "u"), (Texel::Sint, "i")] {
        v.push((format!("cs_fetch_{}", kn), s::cs_fetch(3, k, false, false)));
    }
    v.push((String::from("cs_fetch_f_array"), s::cs_fetch(3, Texel::Float, true, false)));
    v.push((String::from("cs_fetch_depth"), s::cs_fetch(3, Texel::Float, false, true)));
    for (d, dn) in [(SampleDim::D2, "2d"), (SampleDim::D2Array, "2d_array"), (SampleDim::Cube, "cube"),
                    (SampleDim::CubeArray, "cube_array")] {
        v.push((format!("cs_sample_{}", dn), s::cs_sample(3, d)));
    }
    for (f, fname) in STORAGE_FORMATS {
        v.push((format!("cs_storage_write_{}", fname), s::cs_storage_write(3, f)));
        v.push((format!("cs_storage_read_{}", fname), s::cs_storage_read(3, f)));
    }
    for (f, fname) in [(Fmt::R32f, "r32f"), (Fmt::R32ui, "r32ui"), (Fmt::R32i, "r32i")] {
        v.push((format!("cs_storage_rw_{}", fname), s::cs_storage_rw(3, f)));
    }
    v.push((String::from("cs_query"), s::cs_query(3)));
    v.push((String::from("cs_bindless_sample_16"), s::cs_bindless_sample()));
    v
}

/// the storage formats gputest and shadertest try
pub const STORAGE_FORMATS: [(Fmt, &str); 8] = [(Fmt::Rgba8, "rgba8"), (Fmt::Rgba16f, "rgba16f"), (Fmt::R32f, "r32f"),
    (Fmt::Rgba32f, "rgba32f"), (Fmt::R32ui, "r32ui"), (Fmt::Rgba32ui, "rgba32ui"), (Fmt::R32i, "r32i"), (Fmt::Rgba32i, "rgba32i")];

/// the tiny module: an empty GLCompute entry point
pub fn tiny() -> Vec<u32> {
    let mut m = Module::new(3);
    let void = m.ty_void();
    let (main, _) = m.begin_function(void, &[]);
    m.ret();
    m.end_function();
    m.entry_point(Model::GLCompute, main, "main");
    m.exec_mode(main, Mode::LocalSize, &[1, 1, 1]);
    m.finish()
}

/// tiny() word by word from the SPIR-V specification (2.3 physical layout, 2.4 logical
/// layout, 3.x enumerants, the opcode tables of 3.52)
const TINY: [u32; 35] = [
    0x0723_0203, // magic
    0x0001_0300, // version 1.3: 0 | major 1 | minor 3 | 0
    0,           // generator: unregistered
    5,           // bound: ids 1 ..= 4
    0,           // schema
    0x0002_0011, 1, // OpCapability (17), 2 words: Shader (1)
    0x0003_000E, 0, 1, // OpMemoryModel (14): Logical (0), GLSL450 (1)
    0x0005_000F, 5, 2, 0x6E69_616D, 0, // OpEntryPoint (15): GLCompute (5), %2, "main" + NUL word
    0x0006_0010, 2, 17, 1, 1, 1, // OpExecutionMode (16): %2 LocalSize (17) 1 1 1
    0x0002_0013, 1, // OpTypeVoid (19) %1
    0x0003_0021, 3, 1, // OpTypeFunction (33) %3 returning %1
    0x0005_0036, 1, 2, 0, 3, // OpFunction (54): result type %1, %2, control None, type %3
    0x0002_00F8, 4, // OpLabel (248) %4
    0x0001_00FD, // OpReturn (253)
    0x0001_0038, // OpFunctionEnd (56)
];

pub fn golden(t: &mut T) {
    let w = tiny();
    t.eq("tiny module words", w.clone(), TINY.to_vec());
    let bytes = shader::to_bytes(&w);
    t.eq("tiny .spv bytes start", bytes[..8].to_vec(), alloc::vec![0x03, 0x02, 0x23, 0x07, 0x00, 0x03, 0x01, 0x00]);
    t.eq("from_bytes round trip", shader::from_bytes(&bytes), Some(w.clone()));
    let be: Vec<u8> = w.iter().flat_map(|x| x.to_be_bytes()).collect();
    t.eq("from_bytes big endian", shader::from_bytes(&be), Some(w.clone()));
    t.eq("from_bytes no magic", shader::from_bytes(&[0u8; 20]), None);
    t.check("tiny validates", shader::val::validate(&w).is_ok());
}

fn ops_of(w: &[u32]) -> Vec<u16> {
    parse::parse(w).unwrap_or_default().iter().map(|i| i.op).collect()
}

fn caps_of(w: &[u32]) -> Vec<u32> {
    parse::parse(w).unwrap_or_default().iter().filter(|i| i.op == op::Capability).map(|i| i.ops[0]).collect()
}

/// the interface ids of the first entry point
fn interface_of(w: &[u32]) -> Vec<u32> {
    let ins = parse::parse(w).unwrap_or_default();
    let Some(e) = ins.iter().find(|i| i.op == op::EntryPoint) else { return Vec::new() };
    let n = parse::string(&e.ops[2..]).1;
    e.ops[2 + n..].to_vec()
}

pub fn builder(t: &mut T) {
    let mut m = Module::new(3);
    let f = m.ty_f32();
    t.eq("ty_f32 dedup", m.ty_f32(), f);
    let v4 = m.ty_vec(f, 4);
    t.eq("ty_vec dedup", m.ty_vec(f, 4), v4);
    let p = m.ty_ptr(Sc::Function, v4);
    t.eq("ty_ptr dedup", m.ty_ptr(Sc::Function, v4), p);
    t.check("ty_ptr per storage class", m.ty_ptr(Sc::Private, v4) != p);
    let one = m.const_f32(1.0);
    t.eq("const dedup", m.const_f32(1.0), one);
    t.check("-0.0 and 0.0 differ", m.const_f32(-0.0) != m.const_f32(0.0));
    let s1 = m.ty_struct(&[f, v4]);
    t.check("structs are never deduplicated", m.ty_struct(&[f, v4]) != s1);
    let a = m.ty_array(f, 4);
    let a16 = m.ty_array_stride(f, 4, 16);
    t.check("strided array differs from plain", a != a16);
    t.eq("strided array dedup", m.ty_array_stride(f, 4, 16), a16);
    t.check("other stride, other array", m.ty_array_stride(f, 4, 4) != a16);
    let c = m.const_vec_f32(&[1.0, 2.0, 3.0, 4.0]);
    t.eq("composite constant dedup", m.const_vec_f32(&[1.0, 2.0, 3.0, 4.0]), c);
    t.eq("type_of constant", m.type_of(c), v4);

    // capabilities added on the way
    let mut m = Module::new(3);
    m.ty_int(64, false);
    m.ty_float(16);
    let w = m.finish();
    let caps = caps_of(&w);
    t.check("Int64 capability", caps.contains(&(Cap::Int64 as u32)));
    t.check("Float16 capability", caps.contains(&(Cap::Float16 as u32)));
    t.check("Shader capability", caps.contains(&(Cap::Shader as u32)));

    // discard per version, demote's extension, derivative control
    for minor in [3u32, 6] {
        let w = frag_features(minor);
        let ops = ops_of(&w);
        let want = if minor == 6 { op::TerminateInvocation } else { op::Kill };
        t.check(&format!("discard in 1.{}", minor), ops.contains(&want));
        t.check(&format!("DerivativeControl in 1.{}", minor), caps_of(&w).contains(&(Cap::DerivativeControl as u32)));
        let ext = parse::parse(&w).unwrap_or_default().iter().any(|i| i.op == op::Extension);
        t.eq(&format!("demote extension in 1.{}", minor), ext, minor < 6);
        if let Err(e) = shader::val::validate(&w) {
            t.check(&format!("fragment test module 1.{} validates: {}", minor, e[0]), false);
        }
    }

    // entry point interfaces: Input / Output only before 1.4, every used global from 1.4
    let i3 = interface_of(&bench::fs_lambert_spv(3));
    let i6 = interface_of(&bench::fs_lambert_spv(6));
    t.eq("fs_lambert interface 1.3", i3.len(), 3);
    t.eq("fs_lambert interface 1.6", i6.len(), 4);
    t.eq("move interface 1.3 (gl_GlobalInvocationID)", interface_of(&bench::move_spv(3)).len(), 1);
    t.eq("move interface 1.6 (+ objects, pc)", interface_of(&bench::move_spv(6)).len(), 3);

    // NoContraction on the exact shaders only
    let nocontr = |w: &[u32]| {
        parse::parse(w).unwrap_or_default().iter().filter(|i| i.op == op::Decorate && i.ops.get(1) == Some(&42)).count()
    };
    t.check("move: NoContraction", nocontr(&bench::move_spv(3)) > 50);
    t.check("cull: NoContraction", nocontr(&bench::cull_spv(3)) >= 5);
    t.eq("vs: no NoContraction", nocontr(&bench::vs_instanced_spv(3)), 0);

    t.check("docs example validates", shader::val::validate(&doc_example()).is_ok());
    match shader::val::validate(&features()) {
        Ok(()) => t.check("feature module validates", true),
        Err(e) => {
            for x in &e {
                crate::out(&format!("   {}\n", x));
            }
            t.check("feature module validates", false);
        }
    }
}

/// the example of docs/SHADER.md ("Builder-API"), word for word
pub fn doc_example() -> Vec<u32> {
    let mut m = Module::new(shader::BASIS);
    let (void, f, u) = (m.ty_void(), m.ty_f32(), m.ty_u32());
    let arr = m.ty_runtime_array(f, Some(4));
    let block = m.ty_struct_layout("Data", &[("values", arr)], Rule::Std430);
    let data = m.buffer(Sc::StorageBuffer, block, 0, 0);
    let uv3 = m.ty_vec(u, 3);
    let gid = m.builtin(Sc::Input, uv3, Bi::GlobalInvocationId);
    let (main, _) = m.begin_function(void, &[]);
    let g = m.load(gid);
    let i = m.extract(g, &[0]);
    let c0 = m.const_u32(0);
    let p = m.access(data, &[c0, i]);
    let x = m.load(p);
    let y = m.fmul(x, x);
    m.store(p, y);
    m.ret();
    m.end_function();
    m.entry_point(Model::GLCompute, main, "main");
    m.exec_mode(main, Mode::LocalSize, &[64, 1, 1]);
    m.finish()
}

/// a fragment shader with a Fine derivative, demote and discard
pub fn frag_features(minor: u32) -> Vec<u32> {
    let mut m = Module::new(minor);
    let void = m.ty_void();
    let f = m.ty_f32();
    let x = m.input(f, 0);
    let (main, _) = m.begin_function(void, &[]);
    let v = m.load(x);
    m.deriv(op::DPdxFine, v);
    m.demote();
    m.discard();
    m.end_function();
    m.entry_point(Model::Fragment, main, "main");
    m.exec_mode(main, Mode::OriginUpperLeft, &[]);
    m.finish()
}

/// images, atomics, a barrier, a loop with continue in one compute module
pub fn features() -> Vec<u32> {
    let mut m = Module::new(3);
    let void = m.ty_void();
    let f = m.ty_f32();
    let u = m.ty_u32();
    let i = m.ty_i32();
    let v4 = m.ty_vec(f, 4);
    let iv2 = m.ty_vec(i, 2);
    let img = m.ty_image(f, Dim::D2, 0, 0, 0, 2, Fmt::Rgba8);
    let tex = m.ty_image(f, Dim::D2, 0, 0, 0, 1, Fmt::Unknown);
    let smp = m.ty_sampler();
    let iv = m.resource(img, 0, 0);
    let tv = m.resource(tex, 0, 1);
    let sv = m.resource(smp, 0, 2);
    let arr = m.ty_array(u, 64);
    let wg = m.workgroup(arr);
    let uv3 = m.ty_vec(u, 3);
    let gid = m.builtin(Sc::Input, uv3, Bi::LocalInvocationId);
    let (main, _) = m.begin_function(void, &[]);
    let acc = m.local(u, None);
    let g = m.load(gid);
    let x = m.extract(g, &[0]);
    let slot = m.access(wg, &[x]);
    m.store(slot, x);
    m.control_barrier(shader::spv::scope::Workgroup, shader::spv::scope::Workgroup,
                      shader::spv::sem::AcquireRelease | shader::spv::sem::WorkgroupMemory);
    let zero = m.const_u32(0);
    m.store(acc, zero);
    let one = m.const_u32(1);
    m.loop_(|m| { let a = m.load(acc); let k = m.const_u32(4); m.ult(a, k) },
            |m, l| { let a = m.load(acc); let two = m.const_u32(2); let e = m.ieq(a, two); m.if_then(e, |m| m.branch(l.cont)); },
            |m| { let a = m.load(acc); let b = m.iadd(a, one); m.store(acc, b); });
    let r = m.atomic(op::AtomicUMax, slot, shader::spv::scope::Workgroup, 0, one);
    let xi = m.bitcast(i, r);
    let coord = m.construct(iv2, &[xi, xi]);
    let ti = m.load(tv);
    let si = m.load(sv);
    let sampled = m.sampled_image(ti, si);
    let fz = m.const_f32(0.0);
    let uv = m.const_vec_f32(&[0.5, 0.5]);
    let texel = m.sample_lod(sampled, uv, fz);
    let im = m.load(iv);
    let iz = m.const_i32(0);
    let fetched = m.fetch(ti, coord, iz);
    let sum = m.fadd(texel, fetched);
    m.image_write(im, coord, sum);
    let _ = v4;
    m.ret();
    m.end_function();
    m.entry_point(Model::GLCompute, main, "main");
    m.exec_mode(main, Mode::LocalSize, &[64, 1, 1]);
    m.finish()
}

pub fn dis(t: &mut T) {
    let text = dis::disassemble(&tiny()).unwrap_or_default();
    let want = "; SPIR-V\n; Version: 1.3\n; Generator: Khronos; 0\n; Bound: 5\n; Schema: 0\n\
               \x20              OpCapability Shader\n\
               \x20              OpMemoryModel Logical GLSL450\n\
               \x20              OpEntryPoint GLCompute %2 \"main\"\n\
               \x20              OpExecutionMode %2 LocalSize 1 1 1\n\
               \x20      %void = OpTypeVoid\n\
               \x20         %3 = OpTypeFunction %void\n\
               \x20         %2 = OpFunction %void None %3\n\
               \x20         %4 = OpLabel\n\
               \x20              OpReturn\n\
               \x20              OpFunctionEnd\n";
    t.eq("tiny disassembly", text.as_str(), want);
    for (x, s) in [(0.5f32, "0.5"), (1.0, "1"), (core::f32::consts::PI, "3.14159274"), (0.01, "0.00999999978"),
                   (1e-10, "1.00000001e-10"), (-0.0, "-0"), (f32::INFINITY, "0x1p+128"), (-2.5e10, "-2.49999995e+10"),
                   (16777216.0, "16777216"), (123456789.0, "123456792")] {
        t.eq(&format!("float {:e}", x), dis::fmt_float(x).as_str(), s);
    }
    for sh in Shader::ALL {
        for minor in [3u32, 6] {
            let got = dis::disassemble(&sh.build(minor)).unwrap_or_default();
            let want = golden_text(sh, minor).replace('\r', ""); // a checkout with CRLF line ends
            if got != want {
                let line = got.lines().zip(want.lines()).position(|(a, b)| a != b).unwrap_or(0);
                let (g, w) = (got.lines().nth(line).unwrap_or(""), want.lines().nth(line).unwrap_or(""));
                crate::out(&format!("   {} 1.{} line {}: got {:?}, golden {:?}\n", sh.name(), minor, line + 1, g, w));
            }
            t.check(&format!("{} 1.{} disassembly = golden", sh.name(), minor), got == want);
        }
    }
}

/// apps/shadertest/golden/<name>[_16].spvasm (written by --write)
pub fn golden_text(sh: Shader, minor: u32) -> &'static str {
    match (sh, minor) {
        (Shader::Move, 3) => include_str!("../golden/move.spvasm"),
        (Shader::Cull, 3) => include_str!("../golden/cull.spvasm"),
        (Shader::CullReset, 3) => include_str!("../golden/cull_reset.spvasm"),
        (Shader::VsInstanced, 3) => include_str!("../golden/vs_instanced.spvasm"),
        (Shader::VsSingle, 3) => include_str!("../golden/vs_single.spvasm"),
        (Shader::FsLambert, 3) => include_str!("../golden/fs_lambert.spvasm"),
        (Shader::Move, _) => include_str!("../golden/move_16.spvasm"),
        (Shader::Cull, _) => include_str!("../golden/cull_16.spvasm"),
        (Shader::CullReset, _) => include_str!("../golden/cull_reset_16.spvasm"),
        (Shader::VsInstanced, _) => include_str!("../golden/vs_instanced_16.spvasm"),
        (Shader::VsSingle, _) => include_str!("../golden/vs_single_16.spvasm"),
        (Shader::FsLambert, _) => include_str!("../golden/fs_lambert_16.spvasm"),
        _ => "",
    }
}

pub fn layout(t: &mut T) {
    let mut m = Module::new(3);
    let f = m.ty_f32();
    let v2 = m.ty_vec(f, 2);
    let v3 = m.ty_vec(f, 3);
    let v4 = m.ty_vec(f, 4);
    let mat2 = m.ty_mat(v2, 2);
    let mat3 = m.ty_mat(v3, 3);
    let mat4 = m.ty_mat(v4, 4);
    let (s140, s430) = (Rule::Std140, Rule::Std430);
    let offs = |m: &Module, mem: &[Id], r| layout::offsets(m, &mem.iter().map(|x| x.0).collect::<Vec<_>>(), r);
    let size = |m: &Module, ty: Id, r| layout::size(m, ty.0, r);
    // vec3 + float share 16 bytes under both rules
    t.eq("vec3+float std430", offs(&m, &[v3, f], s430), Some(alloc::vec![0, 12]));
    t.eq("vec3+float std140", offs(&m, &[v3, f], s140), Some(alloc::vec![0, 12]));
    t.eq("float+vec3 std430", offs(&m, &[f, v3], s430), Some(alloc::vec![0, 16]));
    t.eq("float+vec2+float std430", offs(&m, &[f, v2, f], s430), Some(alloc::vec![0, 8, 16]));
    // scalar arrays: stride 4 / 16
    let fa = m.ty_array(f, 4);
    t.eq("float[4] stride std430", layout::stride(&m, f.0, s430), Some(4));
    t.eq("float[4] stride std140", layout::stride(&m, f.0, s140), Some(16));
    t.eq("float[4] size std140", size(&m, fa, s140), Some(64));
    t.eq("vec3[] stride std430", layout::stride(&m, v3.0, s430), Some(16));
    // matrices
    t.eq("mat4 size", size(&m, mat4, s430), Some(64));
    t.eq("mat4[3] stride", layout::stride(&m, mat4.0, s140), Some(64));
    let ma = m.ty_array(mat4, 3);
    t.eq("float + mat4[3] std430", offs(&m, &[f, ma], s430), Some(alloc::vec![0, 16]));
    t.eq("mat4[3] size", size(&m, ma, s430), Some(192));
    t.eq("mat3 stride / size std430", (layout::matrix_stride(&m, mat3.0, s430), size(&m, mat3, s430)), (Some(16), Some(48)));
    t.eq("mat2 stride / size std430", (layout::matrix_stride(&m, mat2.0, s430), size(&m, mat2, s430)), (Some(8), Some(16)));
    t.eq("mat2 stride / size std140", (layout::matrix_stride(&m, mat2.0, s140), size(&m, mat2, s140)), (Some(16), Some(32)));
    // nested struct { vec2 b; float c; } between floats
    let inner = m.ty_struct(&[v2, f]);
    t.eq("inner size std430", size(&m, inner, s430), Some(16));
    t.eq("inner size std140", size(&m, inner, s140), Some(16));
    t.eq("nested std430", offs(&m, &[f, inner, f], s430), Some(alloc::vec![0, 8, 24]));
    t.eq("nested std140", offs(&m, &[f, inner, f], s140), Some(alloc::vec![0, 16, 32]));
    let outer = m.ty_struct(&[f, inner, f]);
    t.eq("nested size std430", size(&m, outer, s430), Some(32));
    t.eq("nested size std140", size(&m, outer, s140), Some(48));
    let ia = m.ty_array(inner, 2);
    t.eq("struct array std430", size(&m, ia, s430), Some(32));
    // a runtime array ends a block and has no size
    let ra = m.ty_runtime_array(v4, Some(16));
    t.eq("runtime array last", offs(&m, &[f, ra], s430), Some(alloc::vec![0, 16]));
    t.eq("runtime array not last", offs(&m, &[ra, f], s430), None);
    let b = m.ty_bool();
    t.eq("bool has no layout", layout::size(&m, b.0, s430), None);
    // the benchmark records against the helper
    let obj = m.ty_struct(&[v3, f, v4, v3, f]);
    t.eq("Object offsets", offs(&m, &[v3, f, v4, v3, f], s430), Some(bench::OBJECT_OFF.to_vec()));
    t.eq("Object size", size(&m, obj, s430), Some(bench::OBJECT_SIZE));
    let planes = m.ty_array(v4, 6);
    let cam = m.ty_struct(&[mat4, planes, v3, f]);
    t.eq("Camera offsets", offs(&m, &[mat4, planes, v3, f], s140), Some(bench::CAMERA_OFF.to_vec()));
    t.eq("Camera size", size(&m, cam, s140), Some(bench::CAMERA_SIZE));
    let u = m.ty_u32();
    let i = m.ty_i32();
    t.eq("Push offsets", offs(&m, &[f, u, u, f, u], s430), Some(bench::PUSH_OFF.to_vec()));
    let push = m.ty_struct(&[f, u, u, f, u]);
    t.eq("Push size", size(&m, push, s430), Some(bench::PUSH_SIZE));
    t.eq("DrawCmd offsets", offs(&m, &[u, u, u, i, u], s430), Some(bench::DRAW_OFF.to_vec()));
    let cv = m.ty_struct(&[v4, v4]);
    t.eq("CubeVertex size", size(&m, cv, s430), Some(bench::CUBE_VERTEX_SIZE));
    // what ty_struct_layout writes
    let mat_arr = m.ty_array_layout(mat4, 2, s140);
    let s = m.ty_struct_layout("S", &[("a", f), ("m", mat_arr), ("v", v3)], s140);
    let w = m.finish();
    let ins = parse::parse(&w).unwrap_or_default();
    let md: Vec<(u32, u32, Vec<u32>)> = ins.iter().filter(|x| x.op == op::MemberDecorate && x.ops[0] == s.0)
        .map(|x| (x.ops[1], x.ops[2], x.ops[3..].to_vec())).collect();
    t.check("S.a Offset 0", md.contains(&(0, 35, alloc::vec![0])));
    t.check("S.m Offset 16", md.contains(&(1, 35, alloc::vec![16])));
    t.check("S.m ColMajor", md.contains(&(1, 5, alloc::vec![])));
    t.check("S.m MatrixStride 16", md.contains(&(1, 7, alloc::vec![16])));
    t.check("S.v Offset 144", md.contains(&(2, 35, alloc::vec![144])));
    let stride = ins.iter().any(|x| x.op == op::Decorate && x.ops == [mat_arr.0, 6, 64]);
    t.check("mat4[2] ArrayStride 64", stride);
}

pub fn determinism(t: &mut T) {
    let first: Vec<Vec<u32>> = Shader::ALL.iter().flat_map(|s| [s.build(3), s.build(6)]).collect();
    // the other way round: no state survives between builds
    let mut again: Vec<Vec<u32>> = Shader::ALL.iter().rev().flat_map(|s| [s.build(6), s.build(3)]).collect();
    again.reverse();
    for (k, sh) in Shader::ALL.iter().enumerate() {
        t.check(&format!("{} 1.3 twice equal", sh.name()), first[2 * k] == again[2 * k]);
        t.check(&format!("{} 1.6 twice equal", sh.name()), first[2 * k + 1] == again[2 * k + 1]);
        t.check(&format!("{} 1.3 and 1.6 differ", sh.name()), first[2 * k] != first[2 * k + 1]);
    }
    // FNV-1a over every word: a change of the output shows up here and in the golden files
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for w in first.iter().flatten() {
        for b in w.to_le_bytes() {
            h = (h ^ b as u64).wrapping_mul(0x100_0000_01b3);
        }
    }
    crate::out(&format!("   all shaders: {} words, FNV-1a {:016x}\n", first.iter().map(Vec::len).sum::<usize>(), h));
}

fn write(dir: &str, name: &str, data: &[u8]) -> bool {
    let p = plat::fs::join(dir, name);
    let ok = plat::write_file(&p, data);
    if !ok {
        crate::out(&format!("shadertest: {} cannot be written\n", p));
    }
    ok
}

/// --write DIR
pub fn write_all(dir: &str) -> i32 {
    if !plat::mkdir(dir) {
        crate::out(&format!("shadertest: {} cannot be created\n", dir));
        return 2;
    }
    let mut ok = true;
    for sh in Shader::ALL {
        for minor in [3u32, 6] {
            let w = sh.build(minor);
            let base = if minor == 3 { String::from(sh.name()) } else { format!("{}_16", sh.name()) };
            ok &= write(dir, &format!("{}.spv", base), &shader::to_bytes(&w));
            ok &= write(dir, &format!("{}.spvasm", base), dis::disassemble(&w).unwrap_or_default().as_bytes());
        }
    }
    for c in crate::broken::cases() {
        ok &= write(dir, &format!("bad_{}.spv", c.name), &shader::to_bytes(&c.words));
    }
    ok &= write(dir, "tiny.spv", &shader::to_bytes(&tiny()));
    for (name, w) in samples() {
        ok &= write(dir, &format!("{}.spv", name), &shader::to_bytes(&w));
        ok &= write(dir, &format!("{}.spvasm", name), dis::disassemble(&w).unwrap_or_default().as_bytes());
    }
    if ok { 0 } else { 1 }
}
