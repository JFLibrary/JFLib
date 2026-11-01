// t_wgsl3 -- the subgroup and quad functions of lib/wgsl on the GPU (t_wgsl.rs calls it, at both
// levels; without Features::subgroups_webgpu it reports SKIP)
//
//   subgroups   every function on runtime values against a CPU reference that knows the
//               subgroups: a compute shader writes per invocation its subgroup_invocation_id,
//               subgroup_size, subgroup_id, num_subgroups and the result; the CPU groups the
//               invocations by subgroup_id, the active lanes are those that ran the call.
//               Workgroups of 1 to 256 invocations (partial subgroups), uniform control flow
//               and a branch on the invocation (lanes missing; diagnostic(off,
//               subgroup_uniformity)); a result the spec leaves indeterminate (the lane it
//               selects is not active) is not compared
//   built-ins   ids dense and in range in uniform control flow, the subgroup size within the
//               device's limits, every invocation in exactly one subgroup
//   f16         subgroupAdd on f16 (with shaderSubgroupExtendedTypes)
//   size        @subgroup_size through Device::create_compute_pipeline_sized: the shader sees
//               the required size; with an override through Output::subgroup_size
//   fragment    quad swaps and the broadcast against the pixel coordinates in a fully covered
//               triangle, subgroup_size and the subgroup built-ins in a fragment shader
//   gate        wgsl::feature::for_device without subgroups: `enable subgroups` is an error

use alloc::format;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

use gpu::*;

use crate::{T, out, run_wait};

/// words per invocation: the built-ins, ran, the result
const WORDS: usize = 9;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Ty {
    U32,
    I32,
    F32,
    V2U,
    Bool,
}

impl Ty {
    fn wgsl(self) -> &'static str {
        match self {
            Ty::U32 => "u32",
            Ty::I32 => "i32",
            Ty::F32 => "f32",
            Ty::V2U => "vec2<u32>",
            Ty::Bool => "bool",
        }
    }
    /// x from the input word
    fn from_word(self) -> &'static str {
        match self {
            Ty::U32 => "w",
            Ty::I32 => "bitcast<i32>(w)",
            Ty::F32 => "bitcast<f32>(w)",
            Ty::V2U => "vec2u(w, w ^ 0x55u)",
            Ty::Bool => "(w & 1u) == 1u",
        }
    }
    /// the result r as a vec4u
    fn to_words(self) -> &'static str {
        match self {
            Ty::U32 => "vec4u(r, 0u, 0u, 0u)",
            Ty::I32 | Ty::F32 => "vec4u(bitcast<u32>(r), 0u, 0u, 0u)",
            Ty::V2U => "vec4u(r.x, r.y, 0u, 0u)",
            Ty::Bool => "vec4u(u32(r), 0u, 0u, 0u)",
        }
    }
    /// the input word for a random number (float inputs stay powers of two: exact sums and
    /// products)
    fn input(self, r: u32) -> u32 {
        match self {
            Ty::F32 => [1.0f32, 2.0, -1.0, 0.5][(r & 3) as usize].to_bits(),
            Ty::I32 => r.wrapping_sub(0x8000_0000),
            _ => r,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Op {
    Add,
    Mul,
    Max,
    Min,
    And,
    Or,
    Xor,
    ExAdd,
    InAdd,
    ExMul,
    InMul,
    All,
    Any,
    Ballot,
    First,
    Bcast0,
    Elect,
    Next,
    Up1,
    Down1,
    Xor1,
    Quad0,
    QuadX,
    QuadY,
    QuadD,
}

struct Case {
    expr: &'static str,
    x: Ty,
    ret: Ty,
    op: Op,
    /// the result is defined only when every lane runs the call
    uniform_only: bool,
}

#[rustfmt::skip]
const fn c(expr: &'static str, x: Ty, ret: Ty, op: Op) -> Case {
    Case { expr, x, ret, op, uniform_only: false }
}

#[rustfmt::skip]
const fn u(expr: &'static str, x: Ty, ret: Ty, op: Op) -> Case {
    Case { expr, x, ret, op, uniform_only: true }
}

#[rustfmt::skip]
const CASES: &[Case] = &[
    c("subgroupAdd(x)", Ty::U32, Ty::U32, Op::Add),
    c("subgroupAdd(x)", Ty::I32, Ty::I32, Op::Add),
    c("subgroupAdd(x)", Ty::F32, Ty::F32, Op::Add),
    c("subgroupAdd(x)", Ty::V2U, Ty::V2U, Op::Add),
    c("subgroupExclusiveAdd(x)", Ty::U32, Ty::U32, Op::ExAdd),
    c("subgroupExclusiveAdd(x)", Ty::F32, Ty::F32, Op::ExAdd),
    c("subgroupInclusiveAdd(x)", Ty::I32, Ty::I32, Op::InAdd),
    c("subgroupInclusiveAdd(x)", Ty::F32, Ty::F32, Op::InAdd),
    c("subgroupMul(x)", Ty::U32, Ty::U32, Op::Mul),
    c("subgroupMul(x)", Ty::F32, Ty::F32, Op::Mul),
    c("subgroupExclusiveMul(x)", Ty::I32, Ty::I32, Op::ExMul),
    c("subgroupExclusiveMul(x)", Ty::F32, Ty::F32, Op::ExMul),
    c("subgroupInclusiveMul(x)", Ty::U32, Ty::U32, Op::InMul),
    c("subgroupMax(x)", Ty::U32, Ty::U32, Op::Max),
    c("subgroupMax(x)", Ty::I32, Ty::I32, Op::Max),
    c("subgroupMax(x)", Ty::F32, Ty::F32, Op::Max),
    c("subgroupMin(x)", Ty::U32, Ty::U32, Op::Min),
    c("subgroupMin(x)", Ty::I32, Ty::I32, Op::Min),
    c("subgroupMin(x)", Ty::F32, Ty::F32, Op::Min),
    c("subgroupAnd(x)", Ty::U32, Ty::U32, Op::And),
    c("subgroupAnd(x)", Ty::I32, Ty::I32, Op::And),
    c("subgroupOr(x)", Ty::U32, Ty::U32, Op::Or),
    c("subgroupOr(x)", Ty::V2U, Ty::V2U, Op::Or),
    c("subgroupXor(x)", Ty::U32, Ty::U32, Op::Xor),
    c("subgroupXor(x)", Ty::I32, Ty::I32, Op::Xor),
    c("subgroupAll(x)", Ty::Bool, Ty::Bool, Op::All),
    c("subgroupAny(x)", Ty::Bool, Ty::Bool, Op::Any),
    c("subgroupBallot(x).x", Ty::Bool, Ty::U32, Op::Ballot),
    c("subgroupBroadcastFirst(x)", Ty::U32, Ty::U32, Op::First),
    c("subgroupBroadcastFirst(x)", Ty::V2U, Ty::V2U, Op::First),
    c("subgroupElect()", Ty::U32, Ty::Bool, Op::Elect),
    u("subgroupBroadcast(x, 0u)", Ty::U32, Ty::U32, Op::Bcast0),
    u("subgroupBroadcast(x, 0i)", Ty::F32, Ty::F32, Op::Bcast0),
    u("subgroupShuffle(x, (sii + 1u) % subgroupAdd(1u))", Ty::U32, Ty::U32, Op::Next),
    u("subgroupShuffle(x, i32((sii + 1u) % subgroupAdd(1u)))", Ty::F32, Ty::F32, Op::Next),
    u("subgroupShuffleUp(x, 1u)", Ty::U32, Ty::U32, Op::Up1),
    u("subgroupShuffleDown(x, 1u)", Ty::I32, Ty::I32, Op::Down1),
    u("subgroupShuffleXor(x, 1u)", Ty::U32, Ty::U32, Op::Xor1),
    u("quadBroadcast(x, 0u)", Ty::U32, Ty::U32, Op::Quad0),
    u("quadSwapX(x)", Ty::U32, Ty::U32, Op::QuadX),
    u("quadSwapY(x)", Ty::F32, Ty::F32, Op::QuadY),
    u("quadSwapDiagonal(x)", Ty::V2U, Ty::V2U, Op::QuadD),
];

/// a value of the reference
#[derive(Clone, Copy, PartialEq, Debug)]
enum Val {
    U(u32),
    I(i32),
    F(f32),
    V(u32, u32),
    B(bool),
}

fn val_of(ty: Ty, w: u32) -> Val {
    match ty {
        Ty::U32 => Val::U(w),
        Ty::I32 => Val::I(w as i32),
        Ty::F32 => Val::F(f32::from_bits(w)),
        Ty::V2U => Val::V(w, w ^ 0x55),
        Ty::Bool => Val::B(w & 1 == 1),
    }
}

fn words_of(v: Val) -> [u32; 4] {
    match v {
        Val::U(x) => [x, 0, 0, 0],
        Val::I(x) => [x as u32, 0, 0, 0],
        Val::F(x) => [x.to_bits(), 0, 0, 0],
        Val::V(x, y) => [x, y, 0, 0],
        Val::B(x) => [x as u32, 0, 0, 0],
    }
}

fn add(a: Val, b: Val) -> Val {
    match (a, b) {
        (Val::U(x), Val::U(y)) => Val::U(x.wrapping_add(y)),
        (Val::I(x), Val::I(y)) => Val::I(x.wrapping_add(y)),
        (Val::F(x), Val::F(y)) => Val::F(x + y),
        (Val::V(x, y), Val::V(p, q)) => Val::V(x.wrapping_add(p), y.wrapping_add(q)),
        _ => a,
    }
}

fn mul(a: Val, b: Val) -> Val {
    match (a, b) {
        (Val::U(x), Val::U(y)) => Val::U(x.wrapping_mul(y)),
        (Val::I(x), Val::I(y)) => Val::I(x.wrapping_mul(y)),
        (Val::F(x), Val::F(y)) => Val::F(x * y),
        _ => a,
    }
}

fn bit(a: Val, b: Val, f: fn(u32, u32) -> u32) -> Val {
    match (a, b) {
        (Val::U(x), Val::U(y)) => Val::U(f(x, y)),
        (Val::I(x), Val::I(y)) => Val::I(f(x as u32, y as u32) as i32),
        (Val::V(x, y), Val::V(p, q)) => Val::V(f(x, p), f(y, q)),
        _ => a,
    }
}

fn pick(a: Val, b: Val, max: bool) -> Val {
    let first = match (a, b) {
        (Val::U(x), Val::U(y)) => (x > y) == max,
        (Val::I(x), Val::I(y)) => (x > y) == max,
        (Val::F(x), Val::F(y)) => (x > y) == max,
        _ => true,
    };
    if first { a } else { b }
}

fn identity(ty: Ty, multiply: bool) -> Val {
    let n = multiply as u32;
    match ty {
        Ty::U32 => Val::U(n),
        Ty::I32 => Val::I(n as i32),
        Ty::F32 => Val::F(n as f32),
        Ty::V2U => Val::V(n, n),
        Ty::Bool => Val::B(multiply),
    }
}

/// the reference: lanes = (subgroup_invocation_id, value of x) of the active lanes (ascending);
/// None: the result is indeterminate
fn expected(case: &Case, lanes: &[(u32, Val)], me: u32) -> Option<Val> {
    let x = lanes.iter().find(|l| l.0 == me)?.1;
    let at = |id: u32| lanes.iter().find(|l| l.0 == id).map(|l| l.1);
    let vals = || lanes.iter().map(|l| l.1);
    let fold = |init: Val, f: &dyn Fn(Val, Val) -> Val| vals().fold(init, f);
    let before = |f: &dyn Fn(Val, Val) -> Val, init: Val| lanes.iter().filter(|l| l.0 < me).fold(init, |a, l| f(a, l.1));
    let id0 = identity(case.x, false);
    let id1 = identity(case.x, true);
    Some(match case.op {
        Op::Add => fold(id0, &add),
        Op::Mul => fold(id1, &mul),
        Op::Max => fold(x, &|a, b| pick(a, b, true)),
        Op::Min => fold(x, &|a, b| pick(a, b, false)),
        Op::And => fold(x, &|a, b| bit(a, b, |p, q| p & q)),
        Op::Or => fold(x, &|a, b| bit(a, b, |p, q| p | q)),
        Op::Xor => fold(id0, &|a, b| bit(a, b, |p, q| p ^ q)),
        Op::ExAdd => before(&add, id0),
        Op::InAdd => add(before(&add, id0), x),
        Op::ExMul => before(&mul, id1),
        Op::InMul => mul(before(&mul, id1), x),
        Op::All => Val::B(vals().all(|v| v == Val::B(true))),
        Op::Any => Val::B(vals().any(|v| v == Val::B(true))),
        Op::Ballot => {
            let mut m = 0u32;
            for l in lanes {
                if l.0 < 32 && l.1 == Val::B(true) {
                    m |= 1 << l.0;
                }
            }
            Val::U(m)
        }
        Op::First => lanes.first()?.1,
        Op::Bcast0 => at(0)?,
        Op::Elect => Val::B(lanes.first()?.0 == me),
        Op::Next => at((me + 1) % lanes.len() as u32)?,
        Op::Up1 => at(me.checked_sub(1)?)?,
        Op::Down1 => at(me + 1)?,
        Op::Xor1 => at(me ^ 1)?,
        Op::Quad0 => at(me & !3)?,
        Op::QuadX => at(me ^ 1)?,
        Op::QuadY => at(me ^ 2)?,
        Op::QuadD => at(me ^ 3)?,
    })
}

fn same(ty: Ty, got: &[u32], want: Val) -> bool {
    if ty == Ty::F32 {
        return f32::from_bits(got[0]) == f32::from_bits(words_of(want)[0]);
    }
    got == words_of(want)
}

fn shader(case: &Case, n: u32, branch: bool) -> String {
    let cond = if branch { "li % 3u != 1u" } else { "true" };
    let filter = if branch { "diagnostic(off, subgroup_uniformity);\n" } else { "" };
    format!(
        "enable subgroups;
{filter}@group(0) @binding(0) var<storage, read> inp: array<u32>;
@group(0) @binding(1) var<storage, read_write> outp: array<u32>;
@compute @workgroup_size({n})
fn main(@builtin(local_invocation_index) li: u32, @builtin(subgroup_invocation_id) sii: u32, @builtin(subgroup_size) ss: u32,
        @builtin(subgroup_id) sid: u32, @builtin(num_subgroups) ns: u32) {{
    let w = inp[li];
    let x: {xt} = {conv};
    let k = li * {WORDS}u;
    outp[k] = sii;
    outp[k + 1u] = ss;
    outp[k + 2u] = sid;
    outp[k + 3u] = ns;
    if {cond} {{
        let r: {rt} = {expr};
        outp[k + 4u] = 1u;
        let o = {words};
        outp[k + 5u] = o.x;
        outp[k + 6u] = o.y;
        outp[k + 7u] = o.z;
        outp[k + 8u] = o.w;
    }}
}}",
        xt = case.x.wgsl(),
        conv = case.x.from_word(),
        rt = case.ret.wgsl(),
        expr = case.expr,
        words = case.ret.to_words(),
    )
}

/// the random inputs
fn inputs(ty: Ty, n: u32, seed: u32) -> Vec<u32> {
    let mut s = seed.wrapping_mul(2654435761) | 1;
    (0..n).map(|_| {
        s ^= s << 13;
        s ^= s >> 17;
        s ^= s << 5;
        ty.input(s)
    }).collect()
}

/// one dispatch of a compiled case: the words of every invocation
fn dispatch(t: &mut T, dev: &Device, pipe: &ComputePipeline, bgl: &BindGroupLayout, inp: &[u32], n: u32, what: &str) -> Option<Vec<u32>> {
    let make = |size: usize, t: &mut T| t.res(dev.create_buffer(&BufferDesc { size: size as u64 * 4, usage: BufferUsage::STORAGE, memory: MemoryKind::Device }), "buffer");
    let ib = make(inp.len().max(1), t)?;
    let ob = make(n as usize * WORDS, t)?;
    t.res(dev.queue().write_buffer(&ib, 0, inp), "write")?;
    t.res(dev.queue().write_buffer(&ob, 0, &vec![0u32; n as usize * WORDS]), "write")?;
    let bg = t.res(dev.create_bind_group(&BindGroupDesc { layout: bgl, entries: &[
        BindEntry { binding: 0, resource: Resource::Buffer { buffer: &ib, offset: 0, size: 0 } },
        BindEntry { binding: 1, resource: Resource::Buffer { buffer: &ob, offset: 0, size: 0 } },
    ] }), "bind group")?;
    let mut e = t.res(dev.encoder(), "encoder")?;
    e.set_compute_pipeline(pipe);
    e.set_bind_group(0, &bg);
    e.dispatch(1, 1, 1);
    e.barrier(Access::COMPUTE_WRITE, Access::COPY_READ);
    run_wait(t, dev, e.finish(), what)?;
    let mut got = vec![0u32; n as usize * WORDS];
    t.res(dev.queue().read_buffer(&ob, 0, &mut got), "read")?;
    Some(got)
}

/// compare a dispatch's results with the reference; (compared, wrong, built-in errors)
fn compare(case: &Case, inp: &[u32], got: &[u32], n: u32, branch: bool, lim: &Limits) -> (u32, u32, u32) {
    let (mut compared, mut wrong, mut bad_builtin) = (0, 0, 0);
    let groups: Vec<u32> = (0..n as usize).map(|i| got[i * WORDS + 2]).collect();
    let mut ids = groups.clone();
    ids.sort_unstable();
    ids.dedup();
    let word = |i: usize, k: usize| got[i * WORDS + k];
    for &sid in &ids {
        // the lanes of this subgroup, active or not
        let mut all: Vec<(u32, usize)> = (0..n as usize).filter(|&i| groups[i] == sid).map(|i| (word(i, 0), i)).collect();
        all.sort_unstable();
        // uniform control flow at the start of the shader: ids dense from 0
        if all.iter().enumerate().any(|(k, l)| l.0 != k as u32) {
            bad_builtin += 1;
        }
        for &(_, i) in &all {
            let (ss, ns) = (word(i, 1), word(i, 3));
            if ss < lim.subgroup_min_size.min(4) || ss > lim.subgroup_max_size.max(128) || !ss.is_power_of_two() || word(i, 0) >= ss || sid >= ns {
                bad_builtin += 1;
            }
            if !branch && word(i, 4) != 1 {
                bad_builtin += 1;
            }
        }
        let lanes: Vec<(u32, Val)> = all.iter().filter(|l| word(l.1, 4) == 1).map(|l| (l.0, val_of(case.x, inp[l.1]))).collect();
        for &(lane, i) in &all {
            if word(i, 4) != 1 {
                continue;
            }
            let Some(want) = expected(case, &lanes, lane) else { continue };
            compared += 1;
            if !same(case.ret, &got[i * WORDS + 5..i * WORDS + 9], want) {
                wrong += 1;
            }
        }
    }
    (compared, wrong, bad_builtin)
}

pub fn subgroups(t: &mut T, dev: &Device, minor: u32, lv: &str) {
    let f = *dev.features();
    if !f.subgroups_webgpu {
        out(&format!("  SKIP {lv}: subgroups: the device has no subgroup operations in compute and fragment shaders"));
        return;
    }
    let lim = *dev.limits();
    out(&format!("  {lv}: subgroup size {} ({} .. {}), size control {}, stages {:#x}, operations {:#x}", lim.subgroup_size, lim.subgroup_min_size, lim.subgroup_max_size,
                 f.subgroup_size_control, f.subgroup_stages, f.subgroup_ops));
    let opt = wgsl::CompileOptions { features: wgsl::feature::for_device(f.shader_f16, f.subgroups_webgpu, f.subgroup_size_control), ..wgsl::CompileOptions::new(minor) };
    let Some(bgl) = t.res(dev.create_bind_group_layout(&[
        Binding { binding: 0, kind: BindingKind::StorageBufferRead, stages: Stages::COMPUTE },
        Binding { binding: 1, kind: BindingKind::StorageBuffer, stages: Stages::COMPUTE },
    ]), "layout") else { return };
    let Some(pl) = t.res(dev.create_pipeline_layout(&PipelineLayoutDesc { groups: &[&bgl], push_constants: 0 }), "pipeline layout") else { return };
    let mut sizes_seen: Vec<u32> = Vec::new();
    let (mut total, mut wrong_total, mut builtin_total, mut spv_words) = (0u32, 0u32, 0u32, 0usize);
    for case in CASES {
        let (mut compared, mut wrong, mut bad) = (0, 0, 0);
        for n in [1u32, 3, 7, 32, 33, 64, 100, 256] {
            for branch in [false, true] {
                if branch && case.uniform_only {
                    continue;
                }
                let src = shader(case, n, branch);
                let o = match wgsl::compile(&src, &opt) {
                    Ok(o) => o,
                    Err(d) => {
                        t.fail(&format!("{lv}: {}: compile: {}", case.expr, d.first().map(|x| format!("{x}")).unwrap_or_default()));
                        continue;
                    }
                };
                let Some(e) = o.entry("main") else { continue };
                spv_words += e.words.len();
                let Some(m) = t.res(ShaderModule::from_spirv(dev, &e.words), "module") else { continue };
                let Some(pipe) = t.res(dev.create_compute_pipeline(&ComputePipelineDesc { layout: &pl, shader: ShaderStage::new(&m, "main") }), "pipeline") else { continue };
                let inp = inputs(case.x, n, n * 7 + branch as u32 + 11);
                let Some(got) = dispatch(t, dev, &pipe, &bgl, &inp, n, case.expr) else { continue };
                let (a, b, c) = compare(case, &inp, &got, n, branch, &lim);
                compared += a;
                wrong += b;
                bad += c;
                if !sizes_seen.contains(&got[1]) {
                    sizes_seen.push(got[1]);
                }
            }
        }
        total += compared;
        wrong_total += wrong;
        builtin_total += bad;
        t.ok(wrong == 0 && bad == 0 && compared > 0, &format!("{lv}: {} on {}: {} results = the reference{}", case.expr, case.x.wgsl(), compared,
                                                                if wrong + bad > 0 { format!(" ({wrong} differ, {bad} built-in values wrong)") } else { String::new() }));
    }
    out(&format!("  {lv}: subgroups: {} cases, {total} results compared, {wrong_total} differ, {builtin_total} wrong built-ins, subgroup sizes seen {:?}, {spv_words} SPIR-V words",
                 CASES.len(), sizes_seen));
    f16_add(t, dev, &opt, &pl, &bgl, lv);
    size_control(t, dev, &opt, &pl, &bgl, lv);
    fragment(t, dev, &opt, lv);
    let off = wgsl::CompileOptions { features: wgsl::feature::for_device(f.shader_f16, false, false), ..wgsl::CompileOptions::new(minor) };
    let denied = wgsl::compile("enable subgroups;\n@compute @workgroup_size(1) fn main() { }", &off);
    t.ok(denied.as_ref().err().is_some_and(|d| d.iter().any(|x| x.msg.contains("nicht verfügbar"))), &format!("{lv}: a device without subgroups: `enable subgroups` is an error"));
}

/// subgroupAdd on f16 (shaderSubgroupExtendedTypes)
fn f16_add(t: &mut T, dev: &Device, opt: &wgsl::CompileOptions, pl: &PipelineLayout, bgl: &BindGroupLayout, lv: &str) {
    let f = dev.features();
    if !(f.shader_f16 && f.subgroup_extended_types) {
        out(&format!("  SKIP {lv}: subgroups on f16: the device has no shaderFloat16 + shaderSubgroupExtendedTypes"));
        return;
    }
    let src = "enable f16;
enable subgroups;
@group(0) @binding(0) var<storage, read> inp: array<u32>;
@group(0) @binding(1) var<storage, read_write> outp: array<u32>;
@compute @workgroup_size(64)
fn main(@builtin(local_invocation_index) li: u32, @builtin(subgroup_invocation_id) sii: u32, @builtin(subgroup_size) ss: u32,
        @builtin(subgroup_id) sid: u32, @builtin(num_subgroups) ns: u32) {
    let x = f16(bitcast<f32>(inp[li]));
    let r = subgroupAdd(x);
    let k = li * 9u;
    outp[k] = sii;
    outp[k + 1u] = ss;
    outp[k + 2u] = sid;
    outp[k + 3u] = ns;
    outp[k + 4u] = 1u;
    outp[k + 5u] = bitcast<u32>(f32(r));
}";
    let o = match wgsl::compile(src, opt) {
        Ok(o) => o,
        Err(d) => {
            t.fail(&format!("{lv}: f16: compile: {}", d.first().map(|x| format!("{x}")).unwrap_or_default()));
            return;
        }
    };
    let Some(e) = o.entry("main") else { return };
    t.ok(e.needs.subgroup_extended_types, &format!("{lv}: f16: Needs::subgroup_extended_types"));
    let Some(m) = t.res(ShaderModule::from_spirv(dev, &e.words), "module") else { return };
    let Some(pipe) = t.res(dev.create_compute_pipeline(&ComputePipelineDesc { layout: pl, shader: ShaderStage::new(&m, "main") }), "pipeline") else { return };
    let inp = inputs(Ty::F32, 64, 99);
    let Some(got) = dispatch(t, dev, &pipe, bgl, &inp, 64, "f16") else { return };
    let case = c("", Ty::F32, Ty::F32, Op::Add);
    let (a, b, d) = compare(&case, &inp, &got, 64, false, dev.limits());
    t.ok(a == 64 && b == 0 && d == 0, &format!("{lv}: subgroupAdd on f16: {a} results = the reference ({b} differ, {d} built-ins wrong)"));
}

/// @subgroup_size: the pipeline with the required size; the shader sees it
fn size_control(t: &mut T, dev: &Device, opt: &wgsl::CompileOptions, pl: &PipelineLayout, bgl: &BindGroupLayout, lv: &str) {
    let f = dev.features();
    if !f.subgroup_size_control {
        out(&format!("  SKIP {lv}: @subgroup_size: the device has no subgroup size control"));
        return;
    }
    let lim = dev.limits();
    let mut size = lim.subgroup_min_size;
    while size <= lim.subgroup_max_size && size != 0 {
        let wg = size.max(64) / size * size;
        for (src, given, why) in [
            (format!("enable subgroup_size_control;\n@group(0) @binding(1) var<storage, read_write> outp: array<u32>;\n@compute @workgroup_size({wg}) @subgroup_size({size}) fn main(@builtin(subgroup_size) ss: u32) {{ outp[0] = ss; }}"), None, "const"),
            (format!("enable subgroup_size_control;\noverride want = 4;\n@group(0) @binding(1) var<storage, read_write> outp: array<u32>;\n@compute @workgroup_size({wg}) @subgroup_size(want) fn main(@builtin(subgroup_size) ss: u32) {{ outp[0] = ss; }}"), Some(size as f64), "override"),
        ] {
            let o = match wgsl::compile(&src, opt) {
                Ok(o) => o,
                Err(d) => {
                    t.fail(&format!("{lv}: @subgroup_size: compile: {}", d.first().map(|x| format!("{x}")).unwrap_or_default()));
                    return;
                }
            };
            let Some(e) = o.entry("main") else { return };
            let pairs: Vec<(&str, f64)> = given.iter().map(|g| ("want", *g)).collect();
            let wanted = o.subgroup_size("main", &pairs);
            t.ok(wanted == Ok(Some(size)), &format!("{lv}: Output::subgroup_size {why}: {wanted:?}"));
            let limits = wgsl::Limits { subgroup_min_size: lim.subgroup_min_size, subgroup_max_size: lim.subgroup_max_size, ..wgsl::Limits::WEBGPU };
            t.ok(o.check_pipeline("main", &pairs, &limits).is_ok(), &format!("{lv}: check_pipeline {why}, size {size}"));
            let Some(m) = t.res(ShaderModule::from_spirv(dev, &e.words), "module") else { return };
            let Some(pipe) = t.res(dev.create_compute_pipeline_sized(&ComputePipelineDesc { layout: pl, shader: ShaderStage::new(&m, "main") }, size), "sized pipeline") else { return };
            let Some(got) = dispatch(t, dev, &pipe, bgl, &[0], (wg / 9).max(1), "size") else { return };
            t.ok(got[0] == size, &format!("{lv}: @subgroup_size({size}) as {why}: the shader sees {}", got[0]));
        }
        size *= 2;
    }
    // out of range: an error, no crash
    let Some(m) = t.res(ShaderModule::from_spirv(dev, &wgsl::compile("@group(0) @binding(1) var<storage, read_write> outp: array<u32>;\n@compute @workgroup_size(64) fn main() { outp[0] = 1u; }", opt).map(|o| o.entries[0].words.clone()).unwrap_or_default()), "module") else { return };
    let r = dev.create_compute_pipeline_sized(&ComputePipelineDesc { layout: pl, shader: ShaderStage::new(&m, "main") }, 3);
    t.ok(matches!(r, Err(GpuError::Invalid(_))), &format!("{lv}: a required size of 3 is Invalid"));
}

const FRAGMENT: &str = "enable subgroups;
@group(0) @binding(0) var<storage, read_write> o: array<atomic<u32>>;
@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4f {
    let p = array(vec2f(-1.0, -1.0), vec2f(3.0, -1.0), vec2f(-1.0, 3.0));
    return vec4f(p[i], 0.0, 1.0);
}
@fragment fn fs(@builtin(position) pos: vec4f, @builtin(subgroup_size) ss: u32, @builtin(subgroup_invocation_id) sii: u32) -> @location(0) vec4f {
    let a = vec2u(pos.xy);
    let xs = quadSwapX(a);
    let ys = quadSwapY(a);
    let ds = quadSwapDiagonal(a);
    let b0 = quadBroadcast(a, 0u);
    let n = subgroupAdd(1u);
    let all_true = subgroupAll(true);
    let any_false = subgroupAny(false);
    // only the invocations the rasterizer covers count (helper invocations add nothing here)
    if !(a.x < 64u && a.y < 64u) { return vec4f(); }
    let k = (a.y * 64u + a.x) * 4u;
    var bad = 0u;
    if xs.x != (a.x ^ 1u) || xs.y != a.y { bad |= 1u; }
    if ys.y != (a.y ^ 1u) || ys.x != a.x { bad |= 2u; }
    if ds.x != (a.x ^ 1u) || ds.y != (a.y ^ 1u) { bad |= 4u; }
    if b0.x != (a.x & ~1u) || b0.y != (a.y & ~1u) { bad |= 8u; }
    if n == 0u || n > ss || sii >= ss || !all_true || any_false { bad |= 16u; }
    atomicStore(&o[k], 1u);
    atomicStore(&o[k + 1u], bad);
    atomicStore(&o[k + 2u], ss);
    atomicStore(&o[k + 3u], n);
    return vec4f();
}";

/// quad swaps against the pixel coordinates, subgroup values in a fragment shader
fn fragment(t: &mut T, dev: &Device, opt: &wgsl::CompileOptions, lv: &str) {
    if !dev.features().fragment_stores {
        out(&format!("  SKIP {lv}: subgroups in fragment shaders: no fragment stores"));
        return;
    }
    let o = match wgsl::compile(FRAGMENT, opt) {
        Ok(o) => o,
        Err(d) => {
            t.fail(&format!("{lv}: fragment: compile: {}", d.first().map(|x| format!("{x}")).unwrap_or_default()));
            return;
        }
    };
    let words = |name: &str| o.entry(name).map(|e| e.words.clone()).unwrap_or_default();
    let Some(vm) = t.res(ShaderModule::from_spirv(dev, &words("vs")), "vs") else { return };
    let Some(fm) = t.res(ShaderModule::from_spirv(dev, &words("fs")), "fs") else { return };
    let Some(bgl) = t.res(dev.create_bind_group_layout(&[Binding { binding: 0, kind: BindingKind::StorageBuffer, stages: Stages::FRAGMENT }]), "layout") else { return };
    let Some(pl) = t.res(dev.create_pipeline_layout(&PipelineLayoutDesc { groups: &[&bgl], push_constants: 0 }), "layout") else { return };
    let Some(pipe) = t.res(dev.create_render_pipeline(&RenderPipelineDesc {
        layout: &pl,
        vertex: ShaderStage::new(&vm, "vs"),
        fragment: Some(ShaderStage::new(&fm, "fs")),
        buffers: &[],
        topology: Topology::Triangles,
        cull: Cull::None,
        front_cw: false,
        targets: &[ColorTarget { format: Format::Rgba8Unorm, blend: None, write_mask: 0xF }],
        depth: None,
    }), "pipeline") else { return };
    let count = 64 * 64 * 4;
    let Some(sb) = t.res(dev.create_buffer(&BufferDesc { size: count as u64 * 4, usage: BufferUsage::STORAGE, memory: MemoryKind::Device }), "buffer") else { return };
    let _ = dev.queue().write_buffer(&sb, 0, &vec![0u32; count]);
    let Some(bg) = t.res(dev.create_bind_group(&BindGroupDesc { layout: &bgl, entries: &[BindEntry { binding: 0, resource: Resource::Buffer { buffer: &sb, offset: 0, size: 0 } }] }), "bind group") else { return };
    let Some(tex) = t.res(dev.create_texture(&TextureDesc::d2(64, 64, Format::Rgba8Unorm, TextureUsage::COLOR_TARGET | TextureUsage::COPY_SRC)), "target") else { return };
    let Some(mut e) = t.res(dev.encoder(), "encoder") else { return };
    e.begin_render(&RenderPassDesc { color: &[ColorAttachment { view: tex.view(), load: Load::Clear, clear: [0.0, 0.0, 0.0, 1.0], store: true,
                                                                before: Layout::Undefined, after: Layout::CopySrc }], depth: None });
    e.set_render_pipeline(&pipe);
    e.set_bind_group(0, &bg);
    e.draw(3, 1, 0, 0);
    e.end_render();
    e.barrier(Access::GRAPHICS_WRITE, Access::COPY_READ);
    if run_wait(t, dev, e.finish(), "fragment").is_none() {
        return;
    }
    let mut got = vec![0u32; count];
    if t.res(dev.queue().read_buffer(&sb, 0, &mut got), "read").is_none() {
        return;
    }
    let (mut covered, mut bad, mut sizes) = (0u32, [0u32; 5], Vec::new());
    for k in 0..64 * 64 {
        if got[k * 4] != 1 {
            continue;
        }
        covered += 1;
        for (i, b) in bad.iter_mut().enumerate() {
            if got[k * 4 + 1] >> i & 1 != 0 {
                *b += 1;
            }
        }
        if !sizes.contains(&got[k * 4 + 2]) {
            sizes.push(got[k * 4 + 2]);
        }
    }
    t.ok(covered == 64 * 64, &format!("{lv}: fragment: {covered} pixels ran"));
    t.ok(bad[0] == 0, &format!("{lv}: fragment: quadSwapX = the neighbour in x ({} wrong)", bad[0]));
    t.ok(bad[1] == 0, &format!("{lv}: fragment: quadSwapY = the neighbour in y ({} wrong)", bad[1]));
    t.ok(bad[2] == 0, &format!("{lv}: fragment: quadSwapDiagonal ({} wrong)", bad[2]));
    t.ok(bad[3] == 0, &format!("{lv}: fragment: quadBroadcast(x, 0) = the quad's first pixel ({} wrong)", bad[3]));
    t.ok(bad[4] == 0, &format!("{lv}: fragment: subgroupAdd(1), subgroupAll, subgroupAny and the built-ins in range ({} wrong), subgroup sizes {:?}", bad[4], sizes));
}
