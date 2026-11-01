// unif -- the uniformity analysis (lib/wgsl/uniformity.rs), the diagnostic filters, the subgroup
// and quad front end, the limits of spec 2.4 and how diagnostics read (notes, render,
// suggestions).  Every failing case carries `$` where the diagnostic must be and a part of its
// text; the corpus (tools/wgsl_corpus/unif, diag, limits) has the verdicts Tint gives.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use wgsl::{feature, Options};

use crate::{strip_marker, T};

macro_rules! cs {
    ($body:literal) => {
        concat!("@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) { ", $body, " }")
    };
    ($pre:literal, $body:literal) => {
        concat!($pre, "\n@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) { ", $body, " }")
    };
}

macro_rules! fs {
    ($body:literal) => {
        concat!("@fragment fn fs(@location(0) x: f32) -> @location(0) vec4f { ", $body, " }")
    };
    ($pre:literal, $body:literal) => {
        concat!($pre, "\n@fragment fn fs(@location(0) x: f32) -> @location(0) vec4f { ", $body, " }")
    };
}

const BARRIER: &str = "`workgroupBarrier` darf nur in uniformem";

/// (name, marked source, part of the message): an error at the marker
const FAIL: &[(&str, &str, &str)] = &[
    // ---- control flow
    ("if", cs!("if i == 0u { $workgroupBarrier(); }"), BARRIER),
    ("else", cs!("if i == 0u { } else { $workgroupBarrier(); }"), BARRIER),
    ("else if", cs!("@group(0) @binding(0) var<uniform> u: u32;", "if u == 0u { } else if i == 0u { } else { $workgroupBarrier(); }"), BARRIER),
    ("return in if", cs!("if i == 0u { return; } $workgroupBarrier();"), BARRIER),
    ("return in nested if", cs!("@group(0) @binding(0) var<uniform> u: u32;", "if u == 0u { if i == 0u { return; } } $workgroupBarrier();"), BARRIER),
    ("switch case", cs!("switch i { case 0u { $workgroupBarrier(); } default { } }"), BARRIER),
    ("switch default", cs!("switch i { case 0u { } default { $workgroupBarrier(); } }"), BARRIER),
    ("return in a switch", cs!("switch i { case 0u { return; } default { } } $workgroupBarrier();"), BARRIER),
    ("storageBarrier", cs!("if i == 0u { $storageBarrier(); }"), "`storageBarrier` darf nur"),
    ("textureBarrier", cs!("if i == 0u { $textureBarrier(); }"), "`textureBarrier` darf nur"),
    ("workgroupUniformLoad in control flow", cs!("var<workgroup> w: u32;", "if i == 0u { let x = $workgroupUniformLoad(&w); }"), "`workgroupUniformLoad` darf nur"),
    ("workgroupUniformLoad pointer", cs!("var<workgroup> w: array<u32, 64>;", "let x = workgroupUniformLoad($&w[i]);"), "das Argument `p` von `workgroupUniformLoad` muss uniform sein"),
    // ---- loops
    ("for condition", cs!("for (var k = 0u; k < i; k++) { $workgroupBarrier(); }"), BARRIER),
    ("while condition", cs!("var k = 0u; while k < i { $workgroupBarrier(); k++; }"), BARRIER),
    ("break later in the loop", cs!("loop { $workgroupBarrier(); if i == 0u { break; } }"), BARRIER),
    ("continue before", cs!("for (var k = 0u; k < 8u; k++) { if i == k { continue; } $workgroupBarrier(); }"), BARRIER),
    ("break or return", cs!("loop { if i == 0u { return; } else { break; } } $workgroupBarrier();"), BARRIER),
    ("return in a loop", cs!("loop { if i == 0u { return; } if i == 1u { break; } } $workgroupBarrier();"), BARRIER),
    ("break if", cs!("loop { $workgroupBarrier(); continuing { break if i == 0u; } }"), BARRIER),
    ("continuing after a continue", cs!("loop { if i == 0u { continue; } continuing { $workgroupBarrier(); break if true; } }"), BARRIER),
    // ---- short-circuit
    ("right side of &&", "fn f() -> bool { $workgroupBarrier(); return true; }\n@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) { if i == 0u && f() { } }", BARRIER),
    ("right side of ||", "fn f() -> bool { $workgroupBarrier(); return true; }\n@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) { if i == 0u || f() { } }", BARRIER),
    ("value of ||", cs!("@group(0) @binding(0) var<uniform> u: u32;", "let b = i == 0u || u == 1u; if b { $workgroupBarrier(); }"), BARRIER),
    // ---- let, var, partial assignment
    ("let", cs!("let k = i * 2u; if k == 0u { $workgroupBarrier(); }"), BARRIER),
    ("var assigned in control flow", cs!("var x = 0; if i == 0u { x = 1; } if x == 0 { $workgroupBarrier(); }"), BARRIER),
    ("compound", cs!("var x = 0u; x += i; if x == 0u { $workgroupBarrier(); }"), BARRIER),
    ("member keeps the old value", cs!("struct S { a: u32, b: u32 }", "var v = S(i, i); v.a = 0u; v.b = 0u; if v.a == 0u { $workgroupBarrier(); }"), BARRIER),
    ("component keeps the old value", cs!("var v = vec2u(i); v.x = 0u; v.y = 0u; if v.x == 0u { $workgroupBarrier(); }"), BARRIER),
    ("element keeps the old value", cs!("var a = array<u32, 2>(i, i); a[0] = 0u; if a[0] == 0u { $workgroupBarrier(); }"), BARRIER),
    ("non-uniform index", cs!("var a = array<u32, 4>(); a[i % 4u] = 1u; if a[0] == 0u { $workgroupBarrier(); }"), BARRIER),
    ("partial swizzle view", cs!("var v = vec3u(i); v.yx = vec2u(0u); if v.x == 0u { $workgroupBarrier(); }"), BARRIER),
    ("no components tracked", cs!("var v = vec2u(0u, i); if v.x == 0u { $workgroupBarrier(); }"), BARRIER),
    ("value at a break of a switch", cs!("@group(0) @binding(0) var<uniform> u: u32;", "var x = 0u; switch u { case 0u { x = i; if u == 4u { break; } x = 0u; } default { } } if x == 0u { $workgroupBarrier(); }"), BARRIER),
    ("value at a break of a loop", cs!("@group(0) @binding(0) var<uniform> u: u32;", "var x = 0u; loop { x = i; if u == 4u { break; } x = 0u; if u == 5u { break; } } if x == 0u { $workgroupBarrier(); }"), BARRIER),
    ("value at a break if", cs!("@group(0) @binding(0) var<uniform> u: u32;", "var x = 0u; loop { continuing { x = i; break if u == 1u; } } if x == 0u { $workgroupBarrier(); }"), BARRIER),
    ("value of the last iteration", cs!("@group(0) @binding(0) var<uniform> u: u32;", "var x = 0u; loop { if x == 0u { $workgroupBarrier(); } x = i; if u == 4u { break; } }"), BARRIER),
    ("through a nested loop", cs!("@group(0) @binding(0) var<uniform> u: u32;", "var x = 0u; loop { loop { if x == 0u { $workgroupBarrier(); } if u == 1u { break; } } x = i; if u == 4u { break; } }"), BARRIER),
    ("value at a continue in continuing", cs!("@group(0) @binding(0) var<uniform> u: u32;", "var x = 0u; loop { x = 0u; if u == 0u { x = i; continue; } continuing { if x == 0u { $workgroupBarrier(); } break if u == 3u; } }"), BARRIER),
    ("constant stored in control flow", cs!("var x = 0u; if i == 0u { x = 1u; } if x == 0u { $workgroupBarrier(); }"), BARRIER),
    // ---- sources
    ("local_invocation_id", "@compute @workgroup_size(64) fn main(@builtin(local_invocation_id) l: vec3u) { if l.x == 0u { $workgroupBarrier(); } }", BARRIER),
    ("global_invocation_id", "@compute @workgroup_size(64) fn main(@builtin(global_invocation_id) l: vec3u) { if l.x == 0u { $workgroupBarrier(); } }", BARRIER),
    ("workgroup_index", "@compute @workgroup_size(64) fn main(@builtin(workgroup_index) w: u32) { if w == 0u { $workgroupBarrier(); } }", BARRIER),
    ("struct of mixed inputs", "struct In { @builtin(workgroup_id) w: vec3u, @builtin(local_invocation_index) l: u32 }\n@compute @workgroup_size(64) fn main(x: In) { if x.w.x == 0u { $workgroupBarrier(); } }", BARRIER),
    ("private", cs!("var<private> pv: u32;", "if pv == 0u { $workgroupBarrier(); }"), BARRIER),
    ("read_write storage", cs!("@group(0) @binding(0) var<storage, read_write> b: u32;", "if b == 0u { $workgroupBarrier(); }"), BARRIER),
    ("workgroup", cs!("var<workgroup> w: u32;", "if w == 0u { $workgroupBarrier(); }"), BARRIER),
    ("atomicLoad", cs!("var<workgroup> a: atomic<u32>;", "if atomicLoad(&a) > 4u { $workgroupBarrier(); }"), BARRIER),
    ("read_write texture", cs!("@group(0) @binding(0) var t: texture_storage_2d<r32uint, read_write>;", "if textureLoad(t, vec2u()).x == 0u { $workgroupBarrier(); }"), BARRIER),
    ("select", cs!("let k = select(0u, 1u, i == 0u); if k == 0u { $workgroupBarrier(); }"), BARRIER),
    // ---- derivatives
    ("dpdx", fs!("var c = 0.0; if x > 0.5 { c = $dpdx(x); } return vec4f(c);"), "`dpdx` darf nur in uniformem"),
    ("fwidthCoarse", fs!("var c = 0.0; if x > 0.5 { c = $fwidthCoarse(x); } return vec4f(c);"), "`fwidthCoarse` darf nur"),
    ("textureSample", fs!("@group(0) @binding(0) var t: texture_2d<f32>;\n@group(0) @binding(1) var s: sampler;", "var c = vec4f(); if x > 0.5 { c = $textureSample(t, s, vec2f()); } return c;"), "`textureSample` darf nur"),
    ("textureSampleBias", fs!("@group(0) @binding(0) var t: texture_2d<f32>;\n@group(0) @binding(1) var s: sampler;", "var c = vec4f(); if x > 0.5 { c = $textureSampleBias(t, s, vec2f(), 1.0); } return c;"), "`textureSampleBias` darf nur"),
    ("textureSampleCompare", fs!("@group(0) @binding(0) var t: texture_depth_2d;\n@group(0) @binding(1) var s: sampler_comparison;", "var c = 0.0; if x > 0.5 { c = $textureSampleCompare(t, s, vec2f(), 0.5); } return vec4f(c);"), "`textureSampleCompare` darf nur"),
    ("derivative's result", fs!("let d = dpdx(1.0); var c = 0.0; if d > 0.5 { c = $dpdy(x); } return vec4f(c);"), "`dpdy` darf nur"),
    ("position", "@fragment fn fs(@builtin(position) p: vec4f) -> @location(0) vec4f { var c = 0.0; if p.x > 0.5 { c = $dpdx(p.x); } return vec4f(c); }", "`dpdx` darf nur"),
    // ---- function chains: the message is at the built-in
    ("call site", "fn f() { $workgroupBarrier(); }\n@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) { if i == 0u { f(); } }", BARRIER),
    ("call site chain", "fn h() { $workgroupBarrier(); }\nfn g() { h(); }\n@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) { if i == 0u { g(); } }", BARRIER),
    ("parameter", "fn g(c: u32) { if c == 0u { $workgroupBarrier(); } }\n@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) { g(i); }", BARRIER),
    ("parameter chain", "fn a(c: u32) { if c == 0u { $workgroupBarrier(); } }\nfn b(c: u32) { a(c + 1u); }\n@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) { b(i); }", BARRIER),
    ("early return on a parameter", "fn f(x: u32) { if x == 0u { return; } $workgroupBarrier(); }\n@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) { f(i); }", BARRIER),
    ("result from a parameter", cs!("fn id(x: u32) -> u32 { return x; }", "if id(i) == 0u { $workgroupBarrier(); }"), BARRIER),
    ("result from control flow", cs!("fn f(x: u32) -> u32 { if x == 0u { return 1u; } return 2u; }", "if f(i) == 0u { $workgroupBarrier(); }"), BARRIER),
    ("result from an atomic", cs!("var<workgroup> a: atomic<u32>;\nfn f() -> u32 { return atomicLoad(&a); }", "if f() == 0u { $workgroupBarrier(); }"), BARRIER),
    ("helper never called", "@group(0) @binding(0) var<storage, read_write> rw: u32;\nfn g() { if rw == 0u { $workgroupBarrier(); } }", BARRIER),
    ("the newest requirement of a function", cs!("if i == 0u { workgroupBarrier(); } if i == 1u { $storageBarrier(); }"), "`storageBarrier`"),
    ("callees first", "@group(0) @binding(0) var<storage, read_write> rw: u32;\n@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) { if i == 0u { storageBarrier(); } g(); }\nfn g() { if rw == 0u { $workgroupBarrier(); } }", BARRIER),
    // ---- pointers
    ("pointee required", "fn g(p: ptr<function, u32>) { if *p == 0u { $workgroupBarrier(); } }\n@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) { var x = i; g(&x); }", BARRIER),
    ("non-uniform index in the argument", "fn g(p: ptr<function, u32>) { if *p == 0u { $workgroupBarrier(); } }\n@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) { var a: array<u32, 64>; g(&a[i]); }", BARRIER),
    ("load through a let", cs!("var a: array<u32, 64>; let p = &a[i]; if *p == 0u { $workgroupBarrier(); }"), BARRIER),
    ("the let captures its index", cs!("var a = array<u32, 64>(); var k = i; let p = &a[k]; k = 0u; *p = 1u; if a[0] == 0u { $workgroupBarrier(); }"), BARRIER),
    ("pointee to the result", cs!("fn f(p: ptr<function, u32>) -> u32 { return *p; }", "var x = i; if f(&x) == 0u { $workgroupBarrier(); }"), BARRIER),
    ("old value returned", cs!("fn w(p: ptr<function, u32>) -> u32 { let old = *p; *p = 0u; return old; }", "var a = i; if w(&a) == 0u { $workgroupBarrier(); }"), BARRIER),
    ("parameter stored", cs!("fn w(p: ptr<function, u32>, v: u32) { *p = v; }", "var x = 0u; w(&x, i); if x == 0u { $workgroupBarrier(); }"), BARRIER),
    ("conditional store", cs!("fn w(p: ptr<function, u32>, c: u32) { if c == 0u { *p = 1u; } }", "var x = 0u; w(&x, i); if x == 0u { $workgroupBarrier(); }"), BARRIER),
    ("store after an early return", cs!("fn w(p: ptr<function, u32>, c: u32) { if c == 0u { return; } *p = 1u; }", "var x = 0u; w(&x, i); if x == 0u { $workgroupBarrier(); }"), BARRIER),
    ("pointee copied", cs!("fn w(p: ptr<function, u32>, q: ptr<function, u32>) { *p = *q; }", "var a = i; var b = 0u; w(&b, &a); if b == 0u { $workgroupBarrier(); }"), BARRIER),
    ("pointer call in control flow", cs!("fn f(p: ptr<function, i32>) { *p = 1; }", "var x = 0; if i == 0u { f(&x); } if x == 0 { $workgroupBarrier(); }"), BARRIER),
    ("read in control flow", cs!("fn r(p: ptr<function, u32>) -> u32 { return *p; }", "var x = 0u; if i == 0u { _ = r(&x); } if x == 0u { $workgroupBarrier(); }"), BARRIER),
    ("partial argument", cs!("fn w0(p: ptr<function, u32>) { *p = 0u; }", "var a = array<u32, 2>(i, i); w0(&a[0]); if a[0] == 0u { $workgroupBarrier(); }"), BARRIER),
    ("read-only pointer, non-uniform index", "@group(0) @binding(0) var<storage> buf: array<u32, 64>;\nfn g(p: ptr<storage, u32, read>) { if *p == 0u { $workgroupBarrier(); } }\n@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) { g(&buf[i]); }", BARRIER),
    ("read_write pointer parameter", "fn g(p: ptr<storage, u32, read_write>) { if *p == 0u { $workgroupBarrier(); } }", BARRIER),
    ("workgroup pointer parameter", "fn g(p: ptr<workgroup, u32>) { if *p == 0u { $workgroupBarrier(); } }", BARRIER),
    // ---- filters
    ("the innermost filter", "diagnostic(off, derivative_uniformity);\n@fragment fn fs(@location(0) x: f32) -> @location(0) vec4f { var c = 0.0; if x > 0.5 { @diagnostic(error, derivative_uniformity) { c = $dpdx(x); } } return vec4f(c); }", "`dpdx`"),
    ("barrier ignores the filter", "diagnostic(off, derivative_uniformity);\n@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) { if i == 0u { $workgroupBarrier(); } }", BARRIER),
    ("the filter at the caller does not help", "@group(0) @binding(0) var t: texture_2d<f32>;\n@group(0) @binding(1) var s: sampler;\nfn h() -> vec4f { return $textureSample(t, s, vec2f()); }\n@fragment fn fs(@location(0) x: f32) -> @location(0) vec4f { var c = vec4f(); @diagnostic(off, derivative_uniformity) if x > 0.5 { c = h(); } return c; }", "`textureSample`"),
    ("then filter, else fails", fs!("var c = 0.0; if x > 0.5 @diagnostic(off, derivative_uniformity) { c = dpdx(x); } else { c = $dpdy(x); } return vec4f(c);"), "`dpdy`"),
    ("body filter, condition fails", fs!("var c = 0.0; for (var k = 0; k < 3 && $dpdx(x) > 0.0; k++) @diagnostic(off, derivative_uniformity) { if x > 0.0 { break; } } return vec4f(c);"), "`dpdx`"),
    ("attribute twice", "@diagnostic(off, derivative_uniformity) @diagnostic(off, $derivative_uniformity) fn f() { }", "zwei `@diagnostic`-Attribute"),
    ("conflicting global filters", "diagnostic(off, derivative_uniformity);\ndiagnostic(error, $derivative_uniformity);", "widersprüchliche Diagnose-Filter"),
    ("unreachable code as an error", "diagnostic(error, chromium.unreachable_code);\nfn f() -> i32 { return 1; $let a = 2; }", "unerreichbarer Code"),
];

/// must pass the checker
const PASS: &[(&str, &str)] = &[
    ("after if", cs!("if i == 0u { } workgroupBarrier();")),
    ("uniform buffer", cs!("@group(0) @binding(0) var<uniform> u: u32;", "if u == 0u { workgroupBarrier(); } else { storageBarrier(); }")),
    ("read-only storage", cs!("@group(0) @binding(0) var<storage> b: u32;", "if b == 0u { workgroupBarrier(); }")),
    ("immediate", cs!("var<immediate> im: u32;", "if im == 0u { workgroupBarrier(); }")),
    ("override", cs!("override o: u32 = 1u;", "if o == 0u { workgroupBarrier(); }")),
    ("workgroup_id", "@compute @workgroup_size(64) fn main(@builtin(workgroup_id) w: vec3u) { if w.x == 0u { workgroupBarrier(); } }"),
    ("num_workgroups", "@compute @workgroup_size(64) fn main(@builtin(num_workgroups) n: vec3u) { if n.x == 1u { workgroupBarrier(); } }"),
    ("struct of uniform inputs", "struct In { @builtin(workgroup_id) w: vec3u, @builtin(num_workgroups) n: vec3u }\n@compute @workgroup_size(64) fn main(x: In) { if x.w.x == 0u { workgroupBarrier(); } }"),
    ("after a switch with break", cs!("switch i { case 0u { break; } default { } } workgroupBarrier();")),
    ("after a for loop", cs!("for (var k = 0u; k < i; k++) { } workgroupBarrier();")),
    ("after a while loop", cs!("var k = 0u; while k < i { k++; } workgroupBarrier();")),
    ("after break if", cs!("loop { continuing { break if i == 0u; } } workgroupBarrier();")),
    ("uniform loop", cs!("for (var k = 0u; k < 8u; k++) { workgroupBarrier(); }")),
    ("uniform break", cs!("@group(0) @binding(0) var<uniform> u: u32;", "loop { workgroupBarrier(); if u == 0u { break; } }")),
    ("body always breaks", cs!("loop { workgroupBarrier(); if i == 0u { break; } else { break; } }")),
    ("body only returns", cs!("loop { workgroupBarrier(); if i == 0u { return; } else { return; } }")),
    ("barrier in continuing", cs!("var k = 0u; loop { k++; continuing { workgroupBarrier(); break if k > 4u; } }")),
    ("discard stays {Next}", fs!("var c = 0.0; for (var k = 0; k < 4; k++) { if x > 0.5 { discard; } c += dpdx(x); } return vec4f(c);")),
    ("uniform left of &&", cs!("@group(0) @binding(0) var<uniform> u: u32;\nfn f() -> bool { workgroupBarrier(); return true; }", "if u == 0u && f() { }")),
    ("overwritten", cs!("var x = i; x = 0u; if x == 0u { workgroupBarrier(); }")),
    ("full vector", cs!("var v = vec2u(i); v = vec2u(0u); if v.x == 0u { workgroupBarrier(); }")),
    ("full swizzle view", cs!("var v = vec2u(i); v.yx = vec2u(0u); if v.x == 0u { workgroupBarrier(); }")),
    ("overwritten in continuing", cs!("@group(0) @binding(0) var<uniform> u: u32;", "var x = 0u; loop { if x == 0u { workgroupBarrier(); } if u == 0u { x = i; continue; } if u == 1u { break; } continuing { x = 0u; } }")),
    ("loop counter sum", cs!("var x = 0u; for (var k = 0u; k < 4u; k++) { x += k; } if x == 0u { workgroupBarrier(); }")),
    ("arrayLength", cs!("@group(0) @binding(0) var<storage, read_write> rw: array<u32>;", "if arrayLength(&rw) > 4u { workgroupBarrier(); }")),
    ("textureLoad of read-only", cs!("@group(0) @binding(0) var t: texture_storage_2d<r32uint, read>;", "if textureLoad(t, vec2u()).x == 0u { workgroupBarrier(); }")),
    ("textureDimensions of read_write", cs!("@group(0) @binding(0) var t: texture_storage_2d<r32uint, read_write>;", "if textureDimensions(t).x == 0u { workgroupBarrier(); }")),
    ("workgroupUniformLoad", cs!("var<workgroup> w: u32;", "if workgroupUniformLoad(&w) == 0u { workgroupBarrier(); }")),
    ("sample level", fs!("@group(0) @binding(0) var t: texture_2d<f32>;\n@group(0) @binding(1) var s: sampler;", "var c = vec4f(); if x > 0.5 { c = textureSampleLevel(t, s, vec2f(x), 0.0) + textureSampleGrad(t, s, vec2f(), vec2f(), vec2f()) + textureGather(0, t, s, vec2f()); } return c;")),
    ("derivative of a non-uniform value", fs!("return vec4f(dpdx(x));")),
    ("parameter chain, uniform", "fn a(c: u32) { if c == 0u { workgroupBarrier(); } }\nfn b(c: u32) { a(c + 1u); }\n@compute @workgroup_size(64) fn main() { b(4u); }"),
    ("parameter before the barrier", "fn f(n: u32) { for (var k = 0u; k < n; k++) { } workgroupBarrier(); }\n@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) { f(i); }"),
    ("result of a uniform argument", cs!("fn id(x: u32) -> u32 { return x; }", "if id(5u) == 0u { workgroupBarrier(); }")),
    ("result not from the argument", cs!("fn f(a: u32) -> u32 { return 1u; }", "if f(i) == 0u { workgroupBarrier(); }")),
    ("unused requirement", "fn g(c: u32) { if c == 0u { workgroupBarrier(); } }"),
    ("pointee uniform", "fn g(p: ptr<function, u32>) { if *p == 0u { workgroupBarrier(); } }\n@compute @workgroup_size(64) fn main() { var x = 0u; g(&x); }"),
    ("full pointer assignment", cs!("var x = i; let p = &x; *p = 0u; if x == 0u { workgroupBarrier(); }")),
    ("only the old value is non-uniform", cs!("fn w(p: ptr<function, u32>) -> u32 { let old = *p; *p = 0u; return old; }", "var a = i; let o = w(&a); if a == 0u { workgroupBarrier(); }")),
    ("store of a uniform value", cs!("fn w(p: ptr<function, u32>, v: u32) { *p = v; }", "var x = i; w(&x, 1u); if x == 0u { workgroupBarrier(); }")),
    ("copy of a uniform value", cs!("fn w(p: ptr<function, u32>, q: ptr<function, u32>) { *p = *q; }", "var a = 0u; var b = i; w(&b, &a); if b == 0u { workgroupBarrier(); }")),
    ("uniform pointer's result", "@group(0) @binding(0) var<uniform> ub: array<vec4u, 4>;\nfn g(p: ptr<uniform, vec4u>) -> u32 { return (*p).x; }\n@compute @workgroup_size(64) fn main() { if g(&ub[1]) == 0u { workgroupBarrier(); } }"),
    ("a write through a read_write pointer", "@group(0) @binding(0) var<storage, read_write> b: u32;\nfn g(p: ptr<storage, u32, read_write>) { *p = 0u; }\n@compute @workgroup_size(64) fn main() { g(&b); workgroupBarrier(); }"),
    // filters
    ("off", concat!("diagnostic(off, derivative_uniformity);\n", fs!("var c = 0.0; if x > 0.5 { c = dpdx(x); } return vec4f(c);"))),
    ("filter on a function", "@diagnostic(off, derivative_uniformity) @fragment fn fs(@location(0) x: f32) -> @location(0) vec4f { var c = 0.0; if x > 0.5 { c = dpdx(x); } return vec4f(c); }"),
    ("filter on an if", fs!("var c = 0.0; @diagnostic(off, derivative_uniformity) if x > 0.5 { c = dpdx(x); } else if dpdy(x) > 0.0 { c = 2.0; } return vec4f(c);")),
    ("filter on a block", fs!("var c = 0.0; if x > 0.5 { @diagnostic(off, derivative_uniformity) { c = dpdx(x); } } return vec4f(c);")),
    ("filter on a switch", fs!("var c = 0.0; @diagnostic(off, derivative_uniformity) switch i32(x) { case 0 { c = dpdx(x); } default { c = dpdy(x); } } return vec4f(c);")),
    ("filter on a switch body", fs!("var c = 0.0; switch i32(x) @diagnostic(off, derivative_uniformity) { case 0 { c = dpdx(x); } default { } } return vec4f(c);")),
    ("filter on a loop statement", fs!("var c = 0.0; @diagnostic(off, derivative_uniformity) loop { if x > c { break; } continuing { c += dpdx(x); } } return vec4f(c);")),
    ("filter on a loop body", fs!("var c = 0.0; loop @diagnostic(off, derivative_uniformity) { if c > x { break; } continuing { c += dpdx(x); } } return vec4f(c);")),
    ("filter on continuing", fs!("var c = 0.0; loop { if x > 0.5 { break; } continuing @diagnostic(off, derivative_uniformity) { c += dpdx(x); break if c > 1.0; } } return vec4f(c);")),
    ("filter on a while", fs!("var c = 0.0; @diagnostic(off, derivative_uniformity) while c < x { c += dpdx(x); } return vec4f(c);")),
    ("filter on a for body", fs!("var c = 0.0; for (var k = 0; k < 4; k++) @diagnostic(off, derivative_uniformity) { if x > 0.5 { c += dpdx(x); } } return vec4f(c);")),
    ("filter on a helper", "@group(0) @binding(0) var t: texture_2d<f32>;\n@group(0) @binding(1) var s: sampler;\n@diagnostic(off, derivative_uniformity) fn h() -> vec4f { return textureSample(t, s, vec2f()); }\n@fragment fn fs(@location(0) x: f32) -> @location(0) vec4f { var c = vec4f(); if x > 0.5 { c = h(); } return c; }"),
    ("global duplicates", "diagnostic(off, derivative_uniformity);\ndiagnostic(off, derivative_uniformity);"),
    ("statement and body are different ranges", "fn f() { @diagnostic(off, derivative_uniformity) if true @diagnostic(error, derivative_uniformity) { } }"),
    ("unreachable code off", "diagnostic(off, chromium.unreachable_code);\nfn f() -> i32 { return 1; let a = 2; }"),
];

/// (name, source, the warnings and infos expected: info?, line, part)
const REPORTS: &[(&str, &str, &[(bool, u32, &str)])] = &[
    ("warning", "diagnostic(warning, derivative_uniformity);\n@fragment fn fs(@location(0) x: f32) -> @location(0) vec4f { var c = 0.0; if x > 0.5 { c = dpdx(x); } return vec4f(c); }", &[(false, 2, "`dpdx` darf nur")]),
    ("info", "diagnostic(info, derivative_uniformity);\n@fragment fn fs(@location(0) x: f32) -> @location(0) vec4f { var c = 0.0; if x > 0.5 { c = dpdx(x); } return vec4f(c); }", &[(true, 2, "`dpdx` darf nur")]),
    ("one warning per function", "diagnostic(warning, derivative_uniformity);\n@fragment fn fs(@location(0) x: f32) -> @location(0) vec4f {\nvar c = 0.0;\nif x > 0.5 { c = dpdx(x); }\nif x > 0.25 { c += fwidth(x); }\nreturn vec4f(c); }", &[(false, 5, "`fwidth`")]),
    ("each function its own", "diagnostic(warning, derivative_uniformity);\nvar<private> pv: f32;\nfn a() -> f32 { var c = 0.0; if pv > 0.0 { c = dpdx(1.0); } return c; }\nfn b() -> f32 { var c = 0.0; if pv > 1.0 { c = dpdy(1.0); } return c; }\n@fragment fn fs() -> @location(0) vec4f { return vec4f(a() + b()); }", &[(false, 3, "`dpdx`"), (false, 4, "`dpdy`")]),
    ("a warning hides the function's info", "@fragment fn fs(@location(0) x: f32) -> @location(0) vec4f {\nvar c = 0.0;\nif x > 0.5 @diagnostic(info, derivative_uniformity) { c = dpdx(x); }\nif x > 0.25 @diagnostic(warning, derivative_uniformity) { c += dpdy(x); }\nreturn vec4f(c); }", &[(false, 4, "`dpdy`")]),
    ("info in a helper, warning in the caller", "var<private> pv: f32;\n@diagnostic(info, derivative_uniformity) fn a() -> f32 { var c = 0.0; if pv > 0.0 { c = dpdx(1.0); } return c; }\n@diagnostic(warning, derivative_uniformity) @fragment fn fs(@location(0) x: f32) -> @location(0) vec4f { var c = a(); if x > 0.0 { c += dpdy(x); } return vec4f(c); }", &[(true, 2, "`dpdx`"), (false, 3, "`dpdy`")]),
    ("through a call chain", "@group(0) @binding(0) var t: texture_2d<f32>;\n@group(0) @binding(1) var s: sampler;\n@diagnostic(warning, derivative_uniformity) fn h() -> vec4f { return textureSample(t, s, vec2f()); }\nfn g() -> vec4f { return h(); }\n@fragment fn fs(@location(0) x: f32) -> @location(0) vec4f { var c = vec4f(); if x > 0.5 { c = g(); } return c; }", &[(false, 3, "`textureSample`")]),
    ("through a parameter", "@group(0) @binding(0) var t: texture_2d<f32>;\n@group(0) @binding(1) var s: sampler;\n@diagnostic(info, derivative_uniformity) fn h(c: f32) -> vec4f { var r = vec4f(); if c > 0.0 { r = textureSample(t, s, vec2f()); } return r; }\n@fragment fn fs(@location(0) x: f32) -> @location(0) vec4f { return h(x); }", &[(true, 3, "`textureSample`")]),
    ("no failure, no message", "diagnostic(warning, derivative_uniformity);\n@fragment fn fs(@location(0) x: f32) -> @location(0) vec4f { return vec4f(dpdx(x)); }", &[]),
    ("unreachable as info", "diagnostic(info, chromium.unreachable_code);\nfn f() -> i32 { return 1; let a = 2; }", &[(true, 2, "unerreichbarer Code")]),
    ("unknown rule with a suggestion", "diagnostic(off, derivative_uniformty);", &[(false, 1, "meintest du `derivative_uniformity`")]),
    ("unknown chromium rule", "diagnostic(off, chromium.foo);", &[(false, 1, "unbekannte Diagnose-Regel `chromium.foo`")]),
    ("no warning for other two-part rules", "diagnostic(error, foo.bar);", &[]),
    ("subgroup warning", "enable subgroups;\ndiagnostic(warning, subgroup_uniformity);\n@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {\nif i == 0u { let a = subgroupAdd(1u); } }", &[(false, 4, "`subgroupAdd` darf nur in subgroup-uniformem")]),
    ("subgroup info", "enable subgroups;\ndiagnostic(info, subgroup_uniformity);\n@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {\nif i == 0u { let a = quadSwapX(1u); } }", &[(true, 4, "`quadSwapX`")]),
    ("delta as a warning", "enable subgroups;\ndiagnostic(warning, subgroup_uniformity);\n@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {\nlet a = subgroupShuffleUp(1u, i); }", &[(false, 4, "das Argument `delta` von `subgroupShuffleUp` muss uniform sein")]),
];

/// (name, marked source, part): subgroups
const SG_FAIL: &[(&str, &str, &str)] = &[
    ("control flow", cs!("enable subgroups;", "if i == 0u { let a = $subgroupAdd(1u); }"), "`subgroupAdd` darf nur in subgroup-uniformem"),
    ("reduction of a non-uniform value", cs!("enable subgroups;", "if subgroupBroadcastFirst(i) == 0u { let a = $subgroupAdd(1u); }"), "`subgroupAdd` darf nur"),
    ("barrier in subgroup-uniform control flow", cs!("enable subgroups;", "if subgroupBroadcastFirst(1u) == 0u { $workgroupBarrier(); }"), BARRIER),
    ("subgroup_id and a barrier", "enable subgroups;\n@compute @workgroup_size(64) fn main(@builtin(subgroup_id) sid: u32) { if sid == 0u { $workgroupBarrier(); } }", BARRIER),
    ("subgroup_invocation_id", "enable subgroups;\n@compute @workgroup_size(64) fn main(@builtin(subgroup_invocation_id) s: u32) { if s == 0u { let a = $subgroupAdd(1u); } }", "`subgroupAdd`"),
    ("subgroup_size in a fragment shader", "enable subgroups;\n@fragment fn fs(@builtin(subgroup_size) s: u32) -> @location(0) vec4f { if s == 32u { let a = $subgroupAdd(1u); } return vec4f(); }", "`subgroupAdd`"),
    ("delta", cs!("enable subgroups;", "let a = subgroupShuffleUp(1u, $i);"), "das Argument `delta`"),
    ("mask", cs!("enable subgroups;", "let a = subgroupShuffleXor(1u, $i);"), "das Argument `mask`"),
    ("elect is not uniform", cs!("enable subgroups;", "if subgroupElect() { let a = $subgroupAdd(1u); }"), "`subgroupAdd`"),
    ("a scan is not uniform", cs!("enable subgroups;", "if subgroupInclusiveAdd(1u) == 0u { let a = $subgroupAdd(1u); }"), "`subgroupAdd`"),
    ("no quad result is uniform", cs!("enable subgroups;", "if quadSwapX(1u) == 0u { let a = $subgroupAdd(1u); }"), "`subgroupAdd`"),
    ("derivative in subgroup-uniform control flow", fs!("enable subgroups;", "var c = 0.0; if subgroupAll(true) { c = $dpdx(x); } return vec4f(c);"), "`dpdx`"),
    ("helper in control flow", "enable subgroups;\nfn f() -> u32 { return $subgroupAdd(1u); }\n@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) { if i == 0u { let a = f(); } }", "`subgroupAdd`"),
    ("no enable", cs!("let a = $subgroupAdd(1u);"), "`subgroupAdd` braucht `enable subgroups;`"),
    ("no enable, quad", cs!("let a = $quadSwapX(1u);"), "`quadSwapX` braucht `enable subgroups;`"),
    ("built-in without enable", "@compute @workgroup_size(64) fn main(@$builtin(subgroup_size) a: u32) { }", "`@builtin(subgroup_size)` braucht `enable subgroups;`"),
    ("subgroup_id without enable", "@compute @workgroup_size(64) fn main(@$builtin(subgroup_id) a: u32) { }", "braucht `enable subgroups;`"),
    ("in a vertex shader", "enable subgroups;\n@vertex fn vs() -> @builtin(position) vec4f { let a = $subgroupAdd(1.0f); return vec4f(a); }", "nur in Compute- und Fragment-Shadern"),
    ("built-in in a vertex shader", "enable subgroups;\n@vertex fn vs(@$builtin(subgroup_size) a: u32) -> @builtin(position) vec4f { return vec4f(); }", "keine Eingabe der vertex-Stufe"),
    ("subgroup_id in a fragment shader", "enable subgroups;\n@fragment fn fs(@$builtin(subgroup_id) a: u32) -> @location(0) vec4f { return vec4f(); }", "keine Eingabe der fragment-Stufe"),
    ("built-in of type i32", "enable subgroups;\n@compute @workgroup_size(64) fn main(@$builtin(subgroup_size) a: i32) { }", "muss vom Typ `u32`"),
    ("broadcast id at run time", cs!("enable subgroups;", "let a = subgroupBroadcast(1u, $i);"), "das Argument `id` von `subgroupBroadcast` muss ein const-Ausdruck"),
    ("broadcast id 128", cs!("enable subgroups;", "let a = subgroupBroadcast(1u, $128u);"), "kleiner als 128"),
    ("broadcast id negative", cs!("enable subgroups;", "let a = subgroupBroadcast(1u, $-1i);"), "darf nicht negativ"),
    ("quad id 4", cs!("enable subgroups;", "let a = quadBroadcast(1u, $4u);"), "kleiner als 4"),
    ("shuffle id 128", cs!("enable subgroups;", "let a = subgroupShuffle(1u, $128u);"), "kleiner als 128"),
    ("delta 128", cs!("enable subgroups;", "let a = subgroupShuffleDown(1u, $128u);"), "das Argument `delta`"),
    ("float subgroupAnd", cs!("enable subgroups;", "let a = $subgroupAnd(1.0f);"), "keine passende Überladung"),
    ("bool subgroupAdd", cs!("enable subgroups;", "let a = $subgroupAdd(true);"), "keine passende Überladung"),
    ("must use", cs!("enable subgroups;", "$subgroupAdd(1u);"), "muss benutzt werden"),
    ("not constant", "enable subgroups;\nconst c = $subgroupAdd(1u);", "nicht erlaubt"),
    ("subgroup_size attribute without enable", "enable subgroups;\n@compute @workgroup_size(64) @$subgroup_size(32) fn main() { }", "`enable subgroup_size_control;`"),
    ("subgroup_size attribute on a helper", "enable subgroup_size_control;\n@$subgroup_size(32) fn f() { }", "nur an `@compute`"),
    ("not a power of two", "enable subgroup_size_control;\n@compute @workgroup_size(64) @$subgroup_size(3) fn main() { }", "Zweierpotenz"),
    ("zero", "enable subgroup_size_control;\n@compute @workgroup_size(64) @$subgroup_size(0) fn main() { }", "mindestens 1"),
    ("a float", "enable subgroup_size_control;\n@compute @workgroup_size(64) @$subgroup_size(4.0) fn main() { }", "`i32` oder `u32`"),
    ("twice", "enable subgroup_size_control;\n@compute @workgroup_size(64) @subgroup_size(32) @$subgroup_size(32) fn main() { }", "steht zweimal"),
];

const SG_PASS: &[(&str, &str)] = &[
    ("reduction of a uniform value", cs!("enable subgroups;", "if subgroupBroadcastFirst(1u) == 0u { let a = subgroupAdd(1u); }")),
    ("subgroup_id", "enable subgroups;\n@compute @workgroup_size(64) fn main(@builtin(subgroup_id) sid: u32) { if sid == 0u { let a = subgroupAdd(1u); } }"),
    ("subgroup_size in compute, barrier", "enable subgroups;\n@compute @workgroup_size(64) fn main(@builtin(subgroup_size) s: u32) { if s == 32u { workgroupBarrier(); } }"),
    ("num_subgroups", "enable subgroups;\n@compute @workgroup_size(64) fn main(@builtin(num_subgroups) n: u32) { if n == 1u { workgroupBarrier(); } }"),
    ("uniform buffer", cs!("enable subgroups;\n@group(0) @binding(0) var<uniform> u: u32;", "if u == 0u { let a = subgroupAdd(1u); }")),
    ("workgroup_id", "enable subgroups;\n@compute @workgroup_size(64) fn main(@builtin(workgroup_id) w: vec3u) { if w.x == 0u { let a = subgroupAdd(1u); } }"),
    ("shuffle id", cs!("enable subgroups;", "let a = subgroupShuffle(1u, i);")),
    ("uniform delta", cs!("enable subgroups;\n@group(0) @binding(0) var<uniform> u: u32;", "let a = subgroupShuffleUp(1u, u);")),
    ("reduction as delta", cs!("enable subgroups;", "let a = subgroupShuffleUp(1u, subgroupAdd(1u));")),
    ("subgroup-uniform return", fs!("enable subgroups;", "if subgroupAll(true) { return vec4f(); } return vec4f(subgroupAdd(1.0));")),
    ("off", cs!("enable subgroups;\ndiagnostic(off, subgroup_uniformity);", "if i == 0u { let a = subgroupAdd(1u); } let b = subgroupShuffleXor(1u, i);")),
    ("every function", cs!("enable subgroups;", "let a = subgroupAdd(1u) + subgroupExclusiveAdd(1u) + subgroupInclusiveAdd(1u) + subgroupMul(1u) + subgroupExclusiveMul(1u) + subgroupInclusiveMul(1u) + subgroupMax(1u) + subgroupMin(1u) + subgroupAnd(1u) + subgroupOr(1u) + subgroupXor(1u) + subgroupBallot(true).x + subgroupBroadcast(1u, 3u) + subgroupBroadcastFirst(1u) + subgroupShuffle(1u, 2i) + subgroupShuffleDown(1u, 1u) + subgroupShuffleUp(1u, 1u) + subgroupShuffleXor(1u, 1u) + quadBroadcast(1u, 0u) + quadSwapDiagonal(1u) + quadSwapX(1u) + quadSwapY(1u); let b = subgroupAll(true) && subgroupAny(false) && subgroupElect();")),
    ("size control implies subgroups", "enable subgroup_size_control;\n@compute @workgroup_size(64) @subgroup_size(32) fn main() { let a = subgroupAdd(1u); }"),
    ("abstract arguments", cs!("enable subgroups;", "let a: i32 = subgroupAdd(1); let c: f32 = subgroupAdd(1.0); let e: vec2i = subgroupBroadcastFirst(vec2(1, 2));")),
    ("f16", "enable f16;\nenable subgroups;\n@compute @workgroup_size(1) fn main() { let a = subgroupAdd(1h); }"),
];

fn limit_cases(t: &mut T) {
    let members = |n: usize| {
        let mut s = String::from("struct S {\n");
        for i in 0..n {
            s.push_str(&format!("  m{}: u32,\n", i));
        }
        s.push_str("}\n");
        s
    };
    t.check("1023 members", wgsl::check(&members(1023)).is_ok());
    t.check("1024 members: error", wgsl::check(&members(1024)).err().is_some_and(|d| d[0].msg.contains("1024 Member")));
    let nest = |n: usize| format!("var<private> v: {}u32{};", "array<".repeat(n), ", 1>".repeat(n));
    t.check("depth 15", wgsl::check(&nest(15)).is_ok());
    t.check("depth 16: error", wgsl::check(&nest(16)).err().is_some_and(|d| d[0].msg.contains("zu tief verschachtelt")));
    t.check("a matrix counts 2", wgsl::check(&format!("var<private> v: {}mat2x2f{};", "array<".repeat(13), ", 1>".repeat(13))).is_ok());
    t.check("a matrix counts 2: 16", wgsl::check(&format!("var<private> v: {}mat2x2f{};", "array<".repeat(14), ", 1>".repeat(14))).is_err());
    let nest_structs = |n: usize| {
        let mut s = String::from("struct S0 { a: u32 }\n");
        for i in 1..n {
            s.push_str(&format!("struct S{} {{ a: S{} }}\n", i, i - 1));
        }
        s
    };
    t.check("15 nested structs", wgsl::check(&nest_structs(15)).is_ok());
    t.check("16 nested structs: error", wgsl::check(&nest_structs(16)).is_err());
    t.check("20000 chained aliases: an error, no stack overflow", {
        let mut s = String::from("alias A0 = array<u32, 1>;\n");
        for i in 1..20000 {
            s.push_str(&format!("alias A{} = array<A{}, 1>;\n", i, i - 1));
        }
        wgsl::check(&s).is_err()
    });
    let params = |n: usize| format!("fn f({}) {{ }}", (0..n).map(|i| format!("p{}: u32", i)).collect::<Vec<_>>().join(", "));
    t.check("255 parameters", wgsl::check(&params(255)).is_ok());
    t.check("256 parameters: error", wgsl::check(&params(256)).err().is_some_and(|d| d[0].msg.contains("256 Parameter") && d[0].col == 4));
    let cases = |n: usize| {
        let mut s = String::from("fn f(x: u32) { switch x { ");
        for i in 0..n - 1 {
            s.push_str(&format!("case {}u {{ }} ", i));
        }
        s.push_str("default { } } }");
        s
    };
    t.check("1023 selectors", wgsl::check(&cases(1023)).is_ok());
    t.check("1024 selectors: error", wgsl::check(&cases(1024)).err().is_some_and(|d| d[0].msg.contains("1024 case-Werte")));
    let ifs = |n: usize| format!("fn f(c: bool) {{ {} {} }}", "if c { ".repeat(n), "}".repeat(n));
    t.check("63 nested ifs", wgsl::check(&ifs(63)).is_ok());
    t.check("64 nested ifs: error", wgsl::check(&ifs(64)).err().is_some_and(|d| d[0].msg.contains("zu tief verschachtelt")));
    let private = |n: u32| format!("var<private> v: array<u32, {}>;\n@compute @workgroup_size(1) fn main() {{ v[0] = 1u; }}", n);
    t.check("8192 private bytes", wgsl::check(&private(2048)).is_ok());
    t.check("8196 private bytes: error", wgsl::check(&private(2049)).err().is_some_and(|d| d[0].msg.contains("8196 Byte")));
    t.check("private variables no shader uses do not count", wgsl::check("var<private> v: array<u32, 2049>;\n@compute @workgroup_size(1) fn main() { }").is_ok());
    let function = |n: u32| format!("fn f() {{ var v: array<u32, {}>; v[0] = 1u; }}", n);
    t.check("8192 function bytes", wgsl::check(&function(2048)).is_ok());
    t.check("8196 function bytes: error", wgsl::check(&function(2049)).err().is_some_and(|d| d[0].msg.contains("8196 Byte")));
    let ctor = |n: usize| format!("const a = array({});", alloc::vec!["1"; n].join(", "));
    t.check("2047 elements", wgsl::check(&ctor(2047)).is_ok());
    t.check("2048 elements: error", wgsl::check(&ctor(2048)).err().is_some_and(|d| d[0].msg.contains("2048 Elementen")));
    for (name, src) in [
        ("uniform", "@group(0) @binding(0) var<uniform> v: array<vec4u, 65536>;"),
        ("workgroup", "var<workgroup> v: array<u32, 65536>;"),
        ("value", "fn f() { let a = array<u32, 65536>(); }"),
        ("parameter", "fn f(p: array<u32, 65536>) { }"),
        ("result", "fn f() -> array<u32, 65536> { return array<u32, 65536>(); }"),
    ] {
        t.check(&format!("65536 elements as {}: error", name), wgsl::check(src).err().is_some_and(|d| d[0].msg.contains("65536 Elementen")));
    }
    t.check("100000 elements in a storage buffer", wgsl::check("@group(0) @binding(0) var<storage> v: array<u32, 100000>;").is_ok());
}

fn pipeline_cases(t: &mut T) {
    let pipe = |src: &str, given: &[(&str, f64)]| -> Result<(), String> {
        let o = wgsl::compile(src, &wgsl::CompileOptions::new(shader::VOLL)).map_err(|d| format!("{}", d[0]))?;
        let name = o.entries[0].name.clone();
        o.check_pipeline(&name, given, &wgsl::Limits::WEBGPU)
    };
    let ov = "override a: i32 = 0;\n@compute @workgroup_size(1) fn main() { ";
    for (name, body, part) in [
        ("1 / a", "var x = 1; x = 1 / a; }", "null"),
        ("1 % a", "var x = 1; x = 1 % a; }", "null"),
        ("x / a", "var x = 1; x = x / a; }", "Division durch null"),
        ("x % a", "var x = 1; x = x % a; }", "Division durch null"),
        ("in a let", "let x = 1 / a; }", "null"),
        ("in a dead branch", "if false { var x = 1 / a; } }", "null"),
        ("nested", "var x = 1; x = x + (2 * (1 / a)); }", "null"),
        ("component-wise", "var x = vec2i(1); x = vec2i(1) / vec2i(a); }", "null"),
        ("compound", "var x = 1; x /= a; }", "Division durch null"),
    ] {
        let r = pipe(&format!("{}{}", ov, body), &[]);
        t.check(&format!("override in a body, {}: {:?}", name, r), r.as_ref().err().is_some_and(|m| m.contains(part)));
    }
    t.check("the value decides: 1 / a with a = 2", pipe(&format!("{}var x = 1; x = 1 / a; }}", ov), &[("a", 2.0)]).is_ok());
    t.check("a / x is decided at run time", pipe(&format!("{}var x = 1; x = a / x; }}", ov), &[]).is_ok());
    t.check("x << a", pipe("override a: u32 = 40u;\n@compute @workgroup_size(1) fn main() { var x = 1u; x = x << a; }", &[]).err().is_some_and(|m| m.contains("Schiebeweite")));
    t.check("index out of bounds", pipe("override a: i32 = 5;\n@compute @workgroup_size(1) fn main() { var v = array<i32, 3>(); var x = 1; x = v[a]; }", &[]).err().is_some_and(|m| m.contains("außerhalb")));
    t.check("vector index", pipe("override a: i32 = 5;\n@compute @workgroup_size(1) fn main() { var v = vec3i(); var x = 1; x = v[a]; }", &[]).is_err());
    t.check("negative index", pipe("override a: i32 = -1;\n@compute @workgroup_size(1) fn main() { var v = array<i32, 3>(); var x = 1; x = v[a]; }", &[]).is_err());
    t.check("clamp", pipe("override a: i32 = 5;\n@compute @workgroup_size(1) fn main() { var x = 1; x = clamp(x, a, 0); }", &[]).err().is_some_and(|m| m.contains("Untergrenze")));
    t.check("sqrt", pipe("override a: f32 = -1.0;\n@compute @workgroup_size(1) fn main() { var x = sqrt(a); }", &[]).err().is_some_and(|m| m.contains("sqrt")));
    t.check("in a called helper", pipe("override a: i32 = 0;\nfn h() -> i32 { return 1 / a; }\n@compute @workgroup_size(1) fn main() { _ = h(); }", &[]).err().is_some_and(|m| m.contains("`h`")));
    t.check("not in a helper not called", pipe("override a: i32 = 0;\nfn h() -> i32 { return 1 / a; }\n@compute @workgroup_size(1) fn main() { }", &[]).is_ok());
    t.check("a decided short-circuit", pipe("override a: i32 = 0;\noverride b: bool = false;\n@compute @workgroup_size(1) fn main() { var c = b && (1 / a == 0); }", &[]).is_ok());
    t.check("float conversion saturates", pipe("override a: f32 = 1e30;\n@compute @workgroup_size(1) fn main() { var x = i32(a); }", &[]).is_ok());
    t.check("x / a on floats is fine", pipe("override a: f32 = 0.0;\n@compute @workgroup_size(1) fn main() { var x = 1.0; x = x / a; }", &[]).is_ok());
    // subgroups
    let sg = "enable subgroups;\noverride o: u32 = 200u;\n@compute @workgroup_size(1) fn main() { ";
    t.check("shuffle id from an override", pipe(&format!("{}let a = subgroupShuffle(1u, o); }}", sg), &[]).err().is_some_and(|m| m.contains("kleiner als 128")));
    t.check("delta from an override", pipe(&format!("{}let a = subgroupShuffleUp(1u, o); }}", sg), &[]).is_err());
    t.check("the override's value decides", pipe(&format!("{}let a = subgroupShuffle(1u, o); }}", sg), &[("o", 5.0)]).is_ok());
    let size = |text: &str, given: &[(&str, f64)]| pipe(&format!("enable subgroup_size_control;\n{}", text), given);
    t.check("@subgroup_size: not a multiple", size("@compute @workgroup_size(30) @subgroup_size(32) fn main() { }", &[]).err().is_some_and(|m| m.contains("Vielfaches")));
    t.check("@subgroup_size from an override", size("override w = 32;\n@compute @workgroup_size(64) @subgroup_size(w) fn main() { }", &[]).is_ok());
    t.check("@subgroup_size: an override that is no power of two", size("override w = 33;\n@compute @workgroup_size(66) @subgroup_size(w) fn main() { }", &[]).err().is_some_and(|m| m.contains("Zweierpotenz")));
    t.check("@subgroup_size: outside the limits", size("@compute @workgroup_size(256) @subgroup_size(256) fn main() { }", &[]).err().is_some_and(|m| m.contains("außerhalb")));
    t.check("@subgroup_size through Output", {
        let o = wgsl::compile("enable subgroup_size_control;\noverride w = 8;\n@compute @workgroup_size(64) @subgroup_size(w) fn main() { }", &wgsl::CompileOptions::new(shader::VOLL)).ok();
        o.is_some_and(|o| o.entries[0].subgroup_size == Some(8) && o.subgroup_size("main", &[("w", 16.0)]) == Ok(Some(16)))
    });
}

fn notes(t: &mut T) {
    let d = |src: &str| wgsl::check(src).err().map(|mut v| v.remove(0));
    // the walk back to the source
    let e = d("fn g(c: u32) { if c == 0u { workgroupBarrier(); } }\n@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) { g(i); }");
    let texts: Vec<String> = e.as_ref().map(|e| e.notes.iter().map(|n| n.msg.clone()).collect()).unwrap_or_default();
    t.eq("notes of a parameter", texts.len(), 4);
    t.check("control flow first", texts.first().is_some_and(|m| m.contains("Kontrollfluss hängt hier")));
    t.check("the parameter", texts.get(1).is_some_and(|m| m.contains("der Parameter `c` von `g`")));
    t.check("what is passed", texts.get(2).is_some_and(|m| m.contains("übergeben")));
    t.check("the source", texts.get(3).is_some_and(|m| m.contains("der eingebaute Wert `i` von `main`")));
    t.check("the note positions", e.as_ref().is_some_and(|e| e.notes[1].line == 1 && e.notes[1].col == 6 && e.notes[2].line == 2));
    for (name, src, part) in [
        ("user input", "@fragment fn fs(@location(0) x: f32) -> @location(0) vec4f { var c = 0.0; if x > 0.5 { c = dpdx(x); } return vec4f(c); }", "die Eingabe `x` von `fs`"),
        ("storage", "@group(0) @binding(0) var<storage, read_write> b: u32;\nfn g() { if b == 0u { workgroupBarrier(); } }", "`read_write`-Speicherpuffers `b`"),
        ("workgroup", "var<workgroup> w: u32;\nfn g() { if w == 0u { workgroupBarrier(); } }", "`workgroup`-Variable `w`"),
        ("private", "var<private> w: u32;\nfn g() { if w == 0u { workgroupBarrier(); } }", "privaten Variable `w`"),
        ("a result", "var<workgroup> a: atomic<u32>;\nfn g() { if atomicLoad(&a) == 0u { workgroupBarrier(); } }", "das Ergebnis von `atomicLoad`"),
        ("a pointer", "fn g(p: ptr<storage, u32, read_write>) { if *p == 0u { workgroupBarrier(); } }", "über den Zeiger `p`"),
        ("a pointee", "fn g(p: ptr<function, u32>) { if *p == 0u { workgroupBarrier(); } }\n@compute @workgroup_size(1) fn main(@builtin(local_invocation_index) i: u32) { var x = i; g(&x); }", "zeigt möglicherweise auf einen nicht uniformen Wert"),
        ("a call", "fn f() { workgroupBarrier(); }\n@compute @workgroup_size(1) fn main(@builtin(local_invocation_index) i: u32) { if i == 0u { f(); } }", "über den Aufruf von `f` in `main`"),
        ("a chain of calls", "fn h() { workgroupBarrier(); }\nfn f() { h(); }\n@compute @workgroup_size(1) fn main(@builtin(local_invocation_index) i: u32) { if i == 0u { f(); } }", "indirekt über den Aufruf von `f` in `main`"),
    ] {
        let e = d(src);
        t.check(&format!("note: {}", name), e.as_ref().is_some_and(|e| e.notes.iter().any(|n| n.msg.contains(part))));
    }
    // a long chain stays short and needs no deep recursion
    let mut chain = String::from("fn f0(c: u32) { if c == 0u { workgroupBarrier(); } }\n");
    for k in 1..3000 {
        chain.push_str(&format!("fn f{}(c: u32) {{ f{}(c); }}\n", k, k - 1));
    }
    chain.push_str("@compute @workgroup_size(1) fn main(@builtin(local_invocation_index) i: u32) { f2999(i); }\n");
    let e = d(&chain);
    t.check("a chain of 3000 parameters: found, notes capped", e.as_ref().is_some_and(|e| e.line == 1 && e.notes.len() <= 24));
    let mut chain = String::from("fn f0() { workgroupBarrier(); }\n");
    for k in 1..3000 {
        chain.push_str(&format!("fn f{}() {{ f{}(); }}\n", k, k - 1));
    }
    chain.push_str("@compute @workgroup_size(1) fn main(@builtin(local_invocation_index) i: u32) { if i == 0u { f2999(); } }\n");
    let e = d(&chain);
    t.check("a chain of 3000 calls: one note for the chain", e.as_ref().is_some_and(|e| e.line == 1 && e.notes.iter().filter(|n| n.msg.contains("Aufruf")).count() == 1));
}

fn render(t: &mut T) {
    let src = "fn f() {\n    let a = b;\n}\n";
    let d = wgsl::check(src).err().unwrap_or_default();
    let r = d.first().map(|x| x.render(src)).unwrap_or_default();
    t.check(&format!("render: {}", r), r.contains("2:13: Fehler: `b` ist hier nicht bekannt\n") && r.contains("   2 |     let a = b;\n") && r.contains("     |             ^\n"));
    // a span
    let src = "fn f() { let a = dpdx(unknown_name); }";
    let r = wgsl::check(src).err().unwrap_or_default().first().map(|x| x.render(src)).unwrap_or_default();
    t.check(&format!("render: a span: {}", r), r.contains(&"^".repeat(12)));
    // notes with their lines
    let src = "fn g(c: u32) { if c == 0u { workgroupBarrier(); } }\n@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) { g(i); }";
    let r = wgsl::check(src).err().unwrap_or_default().first().map(|x| x.render(src)).unwrap_or_default();
    t.check(&format!("render: notes: {}", r), r.contains("Hinweis: der Parameter `c` von `g`") && r.matches(" | ").count() >= 6);
    // a very long line is cut around the place
    let src = format!("fn f() {{ {} let z = q; }}", (0..50).map(|k| format!("let a{} = 1;", k)).collect::<Vec<_>>().join(" "));
    let r = wgsl::check(&src).err().unwrap_or_default().first().map(|x| x.render(&src)).unwrap_or_default();
    t.check(&format!("render: cut: {}", r), r.contains("…") && r.lines().all(|l| l.chars().count() < 130) && r.contains("let z = q;"));
    // unusual positions
    let e = wgsl::Diagnostic { line: 99, col: 5, end_col: 6, error: true, info: false, msg: String::from("m"), start: 0, end: 0, notes: Vec::new() };
    t.check("render: a line that does not exist", e.render("x").contains("99:5: Fehler: m"));
    let e = wgsl::Diagnostic { line: 1, col: 5, end_col: 2, error: false, info: true, msg: String::from("m"), start: 0, end: 0, notes: Vec::new() };
    t.check("render: info", e.render("äöü").contains("1:5: Info: m"));
    t.check("Display: info", format!("{}", e).contains("Info: m"));
}

fn suggestions(t: &mut T) {
    for (name, src, part) in [
        ("a local", "fn f() { var counter = 1; counter = countr; }", "meintest du `counter`?"),
        ("a parameter", "fn f(width: u32) -> u32 { return widht; }", "meintest du `width`?"),
        ("a module constant", "const scale = 1;\nfn f() -> i32 { return scael; }", "meintest du `scale`?"),
        ("a function", "fn helper() { }\nfn f() { helpr(); }", "meintest du `helper`?"),
        ("a built-in function", "fn f() { let a = textureSampl(1); }", "meintest du `textureSample`?"),
        ("a type", "var<private> a: vec3<f33>;", "meintest du `f32`?"),
        ("a user type", "struct Light { a: u32 }\nvar<private> a: Lihgt;", "meintest du `Light`?"),
        ("an attribute", "@fragmen fn f() { }", "meintest du `@fragment`?"),
        ("a built-in value", "@fragment fn f(@builtin(positon) p: vec4f) -> @location(0) vec4f { return p; }", "meintest du `position`?"),
        ("an extension", "enable subgroup;", "meintest du `subgroups`?"),
        ("a language feature", "requires pointer_composit_access;", "meintest du `pointer_composite_access`?"),
        ("a struct member", "struct S { width: u32 }\nfn f(s: S) -> u32 { return s.widht; }", "meintest du `width`?"),
        ("an address space", "var<privat> a: u32;", "meintest du `private`?"),
        ("an access mode", "@group(0) @binding(0) var<storage, read_wrte> a: u32;", "meintest du `read_write`?"),
        ("a texel format", "@group(0) @binding(0) var t: texture_storage_2d<rgba8unor, write>;", "meintest du `rgba8unorm`?"),
    ] {
        let r = wgsl::check(src);
        t.check(&format!("suggestion: {}: {:?}", name, r.as_ref().err().and_then(|d| d.first().map(|x| x.msg.clone()))), r.as_ref().err().is_some_and(|d| d.iter().any(|x| x.msg.contains(part))));
    }
    t.check("no suggestion when nothing is close", wgsl::check("fn f() { let a = qqqqqq; }").err().is_some_and(|d| !d[0].msg.contains("meintest")));
}

pub fn run(t: &mut T) {
    for (name, src) in PASS {
        t.pass(name, src);
    }
    for (name, marked, part) in FAIL {
        t.fail(name, marked, part);
    }
    for (name, src, want) in REPORTS {
        let Some(m) = t.pass(name, src) else { continue };
        let mut got: Vec<(bool, u32, String)> = m.warnings.iter().map(|d| (d.info, d.line, d.msg.clone())).collect();
        got.sort_by_key(|x| x.1);
        let ok = got.len() == want.len() && got.iter().zip(want.iter()).all(|(g, w)| g.0 == w.0 && g.1 == w.1 && g.2.contains(w.2));
        t.check(&format!("{}: warnings and infos {:?}", name, got), ok);
    }
    // warnings before an error stay with the error
    let src = "var<private> pv: f32;\n@diagnostic(warning, derivative_uniformity) fn w() -> f32 { var c = 0.0; if pv > 0.0 { c = dpdx(1.0); } return c; }\nfn e() -> f32 { var c = 0.0; if pv > 0.0 { c = dpdx(1.0); } return c; }";
    let r = wgsl::check(src);
    t.check("a warning, then an error", r.as_ref().err().is_some_and(|d| d.iter().any(|x| !x.error && x.line == 2) && d.iter().any(|x| x.error && x.line == 3)));
    // through compile
    let src = "diagnostic(warning, derivative_uniformity);\n@fragment fn fs(@location(0) x: f32) -> @location(0) vec4f { var c = 0.0; if x > 0.5 { c = dpdx(x); } return vec4f(c); }";
    let o = wgsl::compile(src, &wgsl::CompileOptions::new(shader::VOLL));
    t.check("the warning through compile", o.is_ok_and(|o| o.warnings.len() == 1 && !o.warnings[0].error));
    // subgroups
    for (name, src) in SG_PASS {
        t.pass(name, src);
    }
    for (name, marked, part) in SG_FAIL {
        t.fail(name, marked, part);
    }
    // the features
    let off = Options { features: feature::ALL & !feature::SUBGROUPS };
    t.fail_with("no subgroups on the device", "enable $subgroups;", "nicht verfügbar", &off);
    let off = Options { features: feature::ALL & !feature::SUBGROUP_SIZE_CONTROL };
    t.fail_with("no size control on the device", "enable $subgroup_size_control;", "nicht verfügbar", &off);
    let off = Options { features: feature::ALL & !feature::SUBGROUP_ID };
    t.fail_with("no subgroup_id", "requires $subgroup_id;", "nicht verfügbar", &off);
    t.fail_with("no subgroup_id: the built-in", "enable subgroups;\n@compute @workgroup_size(1) fn main(@$builtin(subgroup_id) a: u32) { }", "die Spracherweiterung `subgroup_id`", &off);
    t.eq("for_device: everything", feature::for_device(true, true, true), feature::ALL);
    t.eq("for_device: no f16", feature::for_device(false, true, true), feature::ALL & !feature::F16);
    t.eq("for_device: no subgroups", feature::for_device(true, false, true), feature::ALL & !feature::SUBGROUPS & !feature::SUBGROUP_SIZE_CONTROL);
    t.eq("for_device: no size control", feature::for_device(true, true, false), feature::ALL & !feature::SUBGROUP_SIZE_CONTROL);
    // without subgroup_uniformity: one scope, reductions are not uniform, no barrier-free ride
    let one = Options { features: feature::ALL & !feature::SUBGROUP_UNIFORMITY };
    t.fail_with("one scope: a reduction's result", cs!("enable subgroups;", "if subgroupBroadcastFirst(1u) == 0u { let a = $subgroupAdd(1u); }"), "`subgroupAdd`", &one);
    t.pass_with("one scope: a barrier and subgroup functions together", cs!("enable subgroups;\n@group(0) @binding(0) var<uniform> u: u32;", "if u == 0u { let a = subgroupAdd(1u); workgroupBarrier(); }"), &one);
    limit_cases(t);
    pipeline_cases(t);
    notes(t);
    render(t);
    suggestions(t);
    // strip_marker is exercised through T::fail; a sanity check of the marker's use here
    let (s, l, c) = strip_marker("a\n$b");
    t.eq("marker", (s.as_str(), l, c), ("a\nb", 2, 1));
}
