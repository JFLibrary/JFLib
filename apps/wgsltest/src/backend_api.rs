// backend_api -- the back end's API (wgsl::compile and Output, docs/WGSL.md "Back-End"):
// what an entry point reports (stage, workgroup size, resources with kind and minimum size,
// immediate bytes, stage variables, needed features), the options (robustness: OpArrayLength
// and UMin gone without it; exact: NoContraction on every float operation; levels: the
// interface lists), overrides (SpecIds, Output::constants with derived and hidden values,
// substitution leaves no specialization constant, WebGPU's errors), the pipeline checks
// (limits, storage texture formats, check_interface), the library mode and compile errors;
// the compile time of the six benchmark shaders per level (perf_backend)

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use shader::spv::op;
use wgsl::ir::{IoKind, Stage, TexDim, TexKind};
use wgsl::spirv::{ResourceKind, SpecSource, HIDDEN_BASE, OVERRIDE_BASE};
use wgsl::CompileOptions;

use crate::T;

/// how often an opcode occurs in words, and how many NoContraction decorations there are
fn count_op(words: &[u32], opc: u16) -> usize {
    shader::parse::parse(words).map(|v| v.iter().filter(|i| i.op == opc).count()).unwrap_or(0)
}

fn no_contraction(words: &[u32]) -> usize {
    shader::parse::parse(words).map(|v| v.iter().filter(|i| i.op == op::Decorate && i.ops.get(1) == Some(&42)).count()).unwrap_or(0)
}

/// the number of interface ids of the first OpEntryPoint
fn interface_len(words: &[u32]) -> usize {
    let Ok(v) = shader::parse::parse(words) else { return 0 };
    let Some(e) = v.iter().find(|i| i.op == op::EntryPoint) else { return 0 };
    // model, function, the name (words up to the one with a zero byte), the interface
    let rest = &e.ops[2..];
    let name = rest.iter().position(|w| w.to_le_bytes().contains(&0)).map(|k| k + 1).unwrap_or(rest.len());
    rest.len() - name
}

fn compile(t: &mut T, name: &str, src: &str, opt: &CompileOptions) -> Option<wgsl::Output> {
    match wgsl::compile(src, opt) {
        Ok(o) => {
            t.ok += 1;
            Some(o)
        }
        Err(d) => {
            t.check(&format!("{} compiles: {}", name, d.first().map(|x| format!("{}", x)).unwrap_or_default()), false);
            None
        }
    }
}

const OVR: &str = "@id(5) override a: i32 = 4;
override b: f32 = 1.5;
override c: u32;
override d = f32(a) * b + 0.25;
override unused: f32 = 3.0;
var<workgroup> w: array<f32, c * 2u>;
@group(0) @binding(0) var<storage, read_write> o: array<f32>;
@compute @workgroup_size(c, 2)
fn main(@builtin(local_invocation_index) i: u32) {
    w[i] = d;
    workgroupBarrier();
    o[i] = w[(i + 1u) % (c * 2u)] + f32(a);
}";

pub fn run(t: &mut T) {
    let voll = CompileOptions::new(shader::VOLL);
    let basis = CompileOptions::new(shader::BASIS);
    // what the benchmark shaders report
    if let Some(o) = compile(t, "cull", wgsl::bench::CULL, &voll) {
        let e = &o.entries[0];
        t.eq("cull stage", e.stage, Stage::Compute);
        t.eq("cull workgroup size", e.workgroup_size, Some([256, 1, 1]));
        t.eq("cull immediate bytes", e.immediate_size, 20);
        let r: Vec<(u32, u32, ResourceKind, u64)> = e.resources.iter().map(|r| (r.group, r.binding, r.kind, r.min_size)).collect();
        t.eq("cull resources", r, alloc::vec![
            (0, 0, ResourceKind::UniformBuffer, 176),
            (0, 1, ResourceKind::StorageBuffer { read_only: true }, 48),
            (0, 2, ResourceKind::StorageBuffer { read_only: false }, 4),
            (0, 3, ResourceKind::StorageBuffer { read_only: false }, 20),
        ]);
        t.eq("cull needs nothing", e.needs, wgsl::spirv::Needs::default());
        t.check("cull: robust: OpArrayLength for the three runtime-array indices", count_op(&e.words, op::ArrayLength) == 3);
    }
    if let Some(o) = compile(t, "vs_instanced", wgsl::bench::VS_INSTANCED, &voll) {
        let e = &o.entries[0];
        let ins: Vec<IoKind> = e.inputs.iter().map(|v| v.kind).collect();
        t.eq("vs inputs", ins.len(), 2);
        t.check("vs outputs: position + 2 locations", e.outputs.len() == 3 && matches!(e.outputs[2].kind, IoKind::Location { location: 1, .. }));
        t.eq("vs read-only resources", e.resources.iter().filter(|r| matches!(r.kind, ResourceKind::StorageBuffer { read_only: true })).count(), 3);
    }
    // robustness and exactness
    let plain = CompileOptions { robust: false, ..voll };
    let exact = CompileOptions { exact: true, ..voll };
    if let (Some(a), Some(b), Some(c)) = (compile(t, "move", wgsl::bench::MOVE, &voll), compile(t, "move unrobust", wgsl::bench::MOVE, &plain),
                                          compile(t, "move exact", wgsl::bench::MOVE, &exact)) {
        let (wa, wb, wc) = (&a.entries[0].words, &b.entries[0].words, &c.entries[0].words);
        t.check(&format!("robust off: no OpArrayLength ({} -> {})", count_op(wa, op::ArrayLength), count_op(wb, op::ArrayLength)),
                count_op(wb, op::ArrayLength) == 0 && count_op(wa, op::ArrayLength) > 0);
        t.check(&format!("robust off: fewer words ({} -> {})", wa.len(), wb.len()), wb.len() < wa.len());
        t.eq("contraction allowed: no NoContraction", no_contraction(wa), 0);
        let floats = count_op(wc, op::FAdd) + count_op(wc, op::FSub) + count_op(wc, op::FMul) + count_op(wc, op::VectorTimesScalar);
        t.check(&format!("exact: NoContraction on every float operation ({} of {})", no_contraction(wc), floats),
                no_contraction(wc) >= floats && floats > 0);
    }
    // the levels: 1.3 lists only Input / Output in OpEntryPoint, 1.6 every global
    if let (Some(a), Some(b)) = (compile(t, "cull 1.3", wgsl::bench::CULL, &basis), compile(t, "cull 1.6", wgsl::bench::CULL, &voll)) {
        let n = |w: &[u32]| interface_len(w);
        t.check(&format!("interface 1.3 {} < 1.6 {}", n(&a.entries[0].words), n(&b.entries[0].words)),
                n(&a.entries[0].words) == 1 && n(&b.entries[0].words) == 6);
        t.eq("SPIR-V versions", (a.entries[0].words[1] >> 8 & 0xff, b.entries[0].words[1] >> 8 & 0xff), (3, 6));
    }
    // overrides
    if let Some(o) = compile(t, "overrides", OVR, &voll) {
        let ids: Vec<(String, u32, bool)> = o.overrides.iter().map(|x| (x.name.clone(), x.id, x.has_default)).collect();
        t.eq("override ids", ids, alloc::vec![
            (String::from("a"), 5, true),
            (String::from("b"), OVERRIDE_BASE + 1, true),
            (String::from("c"), OVERRIDE_BASE + 2, false),
            (String::from("d"), OVERRIDE_BASE + 3, true),
            (String::from("unused"), OVERRIDE_BASE + 4, true),
        ]);
        let e = &o.entries[0];
        let srcs: Vec<SpecSource> = e.spec.iter().map(|s| s.1).collect();
        t.check(&format!("spec constants: c, d, a and two hidden ({:?})", srcs), srcs.len() == 5
                && srcs.iter().filter(|s| matches!(s, SpecSource::Expr(_))).count() == 2
                && !srcs.contains(&SpecSource::Override(4)) && !srcs.contains(&SpecSource::Override(1)));
        t.check("hidden SpecIds above HIDDEN_BASE", e.spec.iter().all(|&(id, s)| matches!(s, SpecSource::Override(_)) || id >= HIDDEN_BASE));
        t.eq("no default workgroup size without c", e.workgroup_size, None);
        match o.constants("main", &[("c", 8.0), ("b", 0.5)]) {
            Ok(c) => {
                let get = |s: SpecSource| e.spec.iter().position(|x| x.1 == s).map(|k| c[k].1);
                t.eq("constants: c", get(SpecSource::Override(2)), Some(8.0));
                t.eq("constants: d = 4 * 0.5 + 0.25 (derived, float)", get(SpecSource::Override(3)), Some(2.25));
                t.eq("constants: a default", get(SpecSource::Override(0)), Some(4.0));
                let hidden: Vec<f64> = e.spec.iter().zip(&c).filter(|(s, _)| matches!(s.1, SpecSource::Expr(_))).map(|(_, v)| v.1).collect();
                t.check(&format!("constants: hidden workgroup x = 8, array length = 16 ({:?})", hidden),
                        hidden.contains(&8.0) && hidden.contains(&16.0));
            }
            Err(m) => t.check(&format!("constants: {}", m), false),
        }
        t.eq("workgroup_size with constants", o.workgroup_size("main", &[("c", 8.0)]), Ok([8, 2, 1]));
        for (g, what) in [(&[][..], "c missing"), (&[("c", 1.0), ("a", 2.0)][..], "a by name though it has @id"), (&[("c", 0.0)][..], "workgroup 0"),
                          (&[("c", 1.0), ("x", 1.0)][..], "unknown"), (&[("c", 1.5e10)][..], "u32 range"), (&[("c", 1.0), ("5", 1e10)][..], "i32 range"),
                          (&[("c", 1.0), ("b", f64::NAN)][..], "NaN for f32")] {
            t.check(&format!("constants error: {}", what), o.constants("main", g).is_err());
        }
        t.check("constants by @id number", o.constants("main", &[("c", 1.0), ("5", -3.0)]).is_ok());
        // substituted: no specialization constants, LocalSize from the values
        let given = [("c", 4.0), ("5", 2.0)];
        if let Some(s) = compile(t, "overrides substituted", OVR, &CompileOptions { constants: Some(&given), ..voll }) {
            let w = &s.entries[0].words;
            t.check("substituted: no specialization constant", shader::reflect::spec_constants(w).map(|v| v.is_empty()).unwrap_or(false));
            t.eq("substituted: workgroup size", shader::reflect::workgroup_size(w, "main", &[]).ok().flatten(), Some([4, 2, 1]));
            t.eq("substituted: reported workgroup size", s.entries[0].workgroup_size, Some([4, 2, 1]));
        }
        t.check("substituted without c: an error", wgsl::compile(OVR, &CompileOptions { constants: Some(&[]), ..voll }).is_err());
    }
    // needs
    let needs = |t: &mut T, name: &str, src: &str, opt: &CompileOptions| -> wgsl::spirv::Needs {
        compile(t, name, src, opt).map(|o| o.entries.iter().fold(wgsl::spirv::Needs::default(), |mut a, e| {
            let n = e.needs;
            a.f16 |= n.f16;
            a.storage16 |= n.storage16;
            a.uniform16 |= n.uniform16;
            a.push16 |= n.push16;
            a.uniform_standard_layout |= n.uniform_standard_layout;
            a.demote |= n.demote;
            a.storage_without_format |= n.storage_without_format;
            a.clip_distance |= n.clip_distance;
            a.primitive_index |= n.primitive_index;
            a.sample_rate_shading |= n.sample_rate_shading;
            a
        })).unwrap_or_default()
    };
    let src = crate::coverage::SOURCES.iter().find(|s| s.0 == "f16_everywhere").map(|s| s.1).unwrap_or("");
    let n = needs(t, "f16", src, &voll);
    t.check("f16: f16, storage16, uniform16, push16", n.f16 && n.storage16 && n.uniform16 && n.push16);
    let n = needs(t, "mat2x2 uniform", "@group(0) @binding(0) var<uniform> m: mat2x2f;\n@fragment fn f() -> @location(0) vec4f { return vec4f(m[0], m[1]); }", &voll);
    t.check("uniform mat2x2f: standard layout", n.uniform_standard_layout);
    let n = needs(t, "mat4 uniform", "@group(0) @binding(0) var<uniform> m: mat4x4f;\n@fragment fn f() -> @location(0) vec4f { return m[0]; }", &voll);
    t.check("uniform mat4x4f: std140", !n.uniform_standard_layout);
    let disc = "@fragment fn f(@builtin(position) p: vec4f) -> @location(0) vec4f { if p.x < 1.0 { discard; } return p; }";
    let d6 = needs(t, "discard 1.6", disc, &voll).demote;
    t.check("discard at Voll: demote", d6);
    let d3 = needs(t, "discard 1.3", disc, &basis).demote;
    t.check("discard at Basis: no demote (flag + OpKill)", !d3);
    if let Some(o) = compile(t, "discard 1.3", disc, &basis) {
        t.check("discard at Basis: OpKill, no OpDemoteToHelperInvocation",
                count_op(&o.entries[0].words, op::Kill) == 1 && count_op(&o.entries[0].words, op::DemoteToHelperInvocation) == 0);
    }
    let n = needs(t, "bgra8 storage", "@group(0) @binding(0) var t: texture_storage_2d<bgra8unorm, write>;\n@compute @workgroup_size(1) fn f() { textureStore(t, vec2u(0u), vec4f(1.0)); }", &voll);
    t.check("bgra8unorm storage: without format", n.storage_without_format);
    // resource kinds
    if let Some(o) = compile(t, "texture kinds", "@group(1) @binding(2) var t: texture_depth_2d_array;\n@group(1) @binding(3) var s: sampler_comparison;\n@group(0) @binding(7) var st: texture_storage_3d<rgba16float, read>;\n@compute @workgroup_size(1) fn f() { _ = textureNumLayers(t); _ = s; _ = textureLoad(st, vec3u(0u)); }", &voll) {
        let r: Vec<(u32, u32, ResourceKind)> = o.entries[0].resources.iter().map(|r| (r.group, r.binding, r.kind)).collect();
        t.eq("resource kinds", r, alloc::vec![
            (0, 7, ResourceKind::StorageTexture { dim: TexDim::D3, format: wgsl::ir::TexelFormat::Rgba16float, access: wgsl::ir::Access::Read }),
            (1, 2, ResourceKind::Texture { dim: TexDim::D2Array, kind: TexKind::Depth }),
            (1, 3, ResourceKind::Sampler { comparison: true }),
        ]);
    }
    // the pipeline checks
    let lim = wgsl::Limits::WEBGPU;
    let chk = |src: &str, entry: &str| -> Result<(), String> {
        let o = wgsl::compile(src, &voll).map_err(|_| String::from("compile"))?;
        o.check_pipeline(entry, &[], &lim)
    };
    t.check("limit: 16 x 16 invocations ok", chk("@compute @workgroup_size(16, 16) fn f() {}", "f").is_ok());
    t.check("limit: 17 x 16 invocations", chk("@compute @workgroup_size(17, 16) fn f() {}", "f").is_err());
    t.check("limit: z 65", chk("@compute @workgroup_size(1, 1, 65) fn f() {}", "f").is_err());
    t.check("limit: workgroup storage 16384 ok", chk("var<workgroup> w: array<f32, 4096>;\n@compute @workgroup_size(1) fn f() { w[0] = 1.0; }", "f").is_ok());
    t.check("limit: workgroup storage 16388", chk("var<workgroup> w: array<f32, 4096>;\nvar<workgroup> x: f32;\n@compute @workgroup_size(1) fn f() { w[0] = x; }", "f").is_err());
    t.check("limit: fragment location 7 ok", chk("@fragment fn f() -> @location(7) vec4f { return vec4f(); }", "f").is_ok());
    t.check("limit: fragment location 8", chk("@fragment fn f() -> @location(8) vec4f { return vec4f(); }", "f").is_err());
    t.check("limit: read_write r32float ok", chk("@group(0) @binding(0) var t: texture_storage_2d<r32float, read_write>;\n@compute @workgroup_size(1) fn f() { textureStore(t, vec2u(), vec4f()); }", "f").is_ok());
    t.check("limit: read_write rgba8unorm", chk("@group(0) @binding(0) var t: texture_storage_2d<rgba8unorm, read_write>;\n@compute @workgroup_size(1) fn f() { textureStore(t, vec2u(), vec4f()); }", "f").is_err());
    let vf = "struct V { @builtin(position) p: vec4f, @location(0) a: vec3f, @location(2) @interpolate(flat) b: i32 }
@vertex fn vs() -> V { return V(vec4f(), vec3f(), 1); }
@fragment fn ok(@location(0) a: vec2f, @location(2) @interpolate(flat) b: i32) -> @location(0) vec4f { return vec4f(a, 0.0, f32(b)); }
@fragment fn missing(@location(1) a: f32) -> @location(0) vec4f { return vec4f(a); }
@fragment fn kind(@location(0) @interpolate(flat) a: vec3i) -> @location(0) vec4f { return vec4f(vec3f(a), 1.0); }
@fragment fn interp(@location(0) @interpolate(linear) a: vec3f) -> @location(0) vec4f { return vec4f(a, 1.0); }";
    if let Some(o) = compile(t, "interfaces", vf, &voll) {
        let vs = o.entry("vs");
        let ck = |n: &str| match (vs, o.entry(n)) {
            (Some(v), Some(f)) => wgsl::check_interface(&o.module, v, f).is_ok(),
            _ => false,
        };
        t.check("interface: fewer components, flat i32 ok", ck("ok"));
        t.check("interface: location without vertex output", !ck("missing"));
        t.check("interface: component type", !ck("kind"));
        t.check("interface: interpolation", !ck("interp"));
    }
    // the library mode and errors
    match wgsl::spirv::compile_library("fn f(p: ptr<function, i32>) -> i32 { *p += 1; return *p * 2; }", &voll) {
        Ok(o) => t.check("library: one module with the function", o.entries.len() == 1 && count_op(&o.entries[0].words, op::Function) == 2),
        Err(_) => t.check("library compiles", false),
    }
    t.check("unknown entry point: an error", wgsl::compile(wgsl::bench::CULL, &CompileOptions { entry: Some("nope"), ..voll }).is_err());
    t.check("one entry point by name", wgsl::compile(vf, &CompileOptions { entry: Some("ok"), ..voll }).is_ok_and(|o| o.entries.len() == 1));
    t.check("SPIR-V 1.7: an error", wgsl::compile(wgsl::bench::CULL, &CompileOptions::new(7)).is_err());
}

/// the compile time of the six benchmark shaders per level (check + lower)
pub fn perf_backend(t: &mut T) {
    for minor in [shader::BASIS, shader::VOLL] {
        let opt = CompileOptions::new(minor);
        let rounds = 20;
        let t0 = plat::time_us();
        let mut words = 0;
        for _ in 0..rounds {
            for (_, s) in wgsl::bench::ALL {
                if let Ok(o) = wgsl::compile(s, &opt) {
                    words += o.entries.iter().map(|e| e.words.len()).sum::<usize>();
                }
            }
        }
        let us = (plat::time_us() - t0) as f64 / rounds as f64;
        let t1 = plat::time_us();
        let ms: Vec<wgsl::Module> = wgsl::bench::ALL.iter().filter_map(|(_, s)| wgsl::check(s).ok()).collect();
        let t2 = plat::time_us();
        for _ in 0..rounds {
            for m in &ms {
                let _ = wgsl::compile_module(m, &opt);
            }
        }
        let lower = (plat::time_us() - t2) as f64 / rounds as f64;
        let _ = t1;
        crate::out(&format!("perf: the six benchmark shaders compiled to SPIR-V 1.{}: {:.0} us per round (lowering alone {:.0} us), \
                             {} words\n", minor, us, lower, words / rounds));
        t.check(&format!("1.{}: six shaders compile", minor), words > 0);
    }
}
