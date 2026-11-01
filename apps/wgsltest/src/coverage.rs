// coverage -- WGSL modules for the back end's spirv group beyond the corpus and the check
// cases: every texture overload family, the stage built-ins and interpolations, f16 in every
// address space and at the stage interface, pointers into every address space as parameters,
// control flow corners, overrides everywhere, robustness corners; each compiles at both levels
// and must pass lib/shader's validator and spirv-val (backend.rs)

pub const SOURCES: &[(&str, &str)] = &[
    ("textures_sample", "@group(0) @binding(0) var s: sampler;
@group(0) @binding(1) var sc: sampler_comparison;
@group(0) @binding(2) var t1: texture_1d<f32>;
@group(0) @binding(3) var t2: texture_2d<f32>;
@group(0) @binding(4) var t2a: texture_2d_array<f32>;
@group(0) @binding(5) var t3: texture_3d<f32>;
@group(0) @binding(6) var tc: texture_cube<f32>;
@group(0) @binding(7) var tca: texture_cube_array<f32>;
@group(0) @binding(8) var d2: texture_depth_2d;
@group(0) @binding(9) var d2a: texture_depth_2d_array;
@group(0) @binding(10) var dc: texture_depth_cube;
@group(0) @binding(11) var dca: texture_depth_cube_array;
@group(0) @binding(12) var ti: texture_2d<i32>;
@group(0) @binding(13) var tu: texture_2d_array<u32>;
@group(0) @binding(14) var ext: texture_external;
@fragment fn main(@location(0) uv: vec2f, @location(1) @interpolate(flat) l: i32) -> @location(0) vec4f {
    var c = textureSample(t1, s, uv.x) + textureSample(t2, s, uv) + textureSample(t2, s, uv, vec2i(1, -1));
    c += textureSample(t2a, s, uv, l) + textureSample(t2a, s, uv, 2u, vec2i(0, 1)) + textureSample(t3, s, vec3f(uv, 0.5));
    c += textureSample(t3, s, vec3f(uv, 0.5), vec3i(1)) + textureSample(tc, s, vec3f(uv, 1.0)) + textureSample(tca, s, vec3f(uv, 1.0), l);
    var d = textureSample(d2, s, uv) + textureSample(d2, s, uv, vec2i(1)) + textureSample(d2a, s, uv, l) + textureSample(dc, s, vec3f(uv, 1.0));
    d += textureSample(dca, s, vec3f(uv, 1.0), 1u) + textureSample(d2a, s, uv, l, vec2i(2));
    c += textureSampleBias(t2, s, uv, 0.5) + textureSampleBias(t2a, s, uv, l, 1.0, vec2i(1)) + textureSampleBias(t3, s, vec3f(uv, 0.0), 0.5);
    c += textureSampleBias(tc, s, vec3f(uv, 1.0), 0.25) + textureSampleBias(tca, s, vec3f(uv, 1.0), 3, 0.5);
    c += textureSampleLevel(t1, s, uv.x, 0.0) + textureSampleLevel(t2, s, uv, 1.0, vec2i(-1)) + textureSampleLevel(t2a, s, uv, l, 2.0);
    c += textureSampleLevel(t3, s, vec3f(uv, 0.0), 0.0) + textureSampleLevel(tc, s, vec3f(uv, 1.0), 1.0);
    c += textureSampleLevel(tca, s, vec3f(uv, 1.0), l, 1.0);
    d += textureSampleLevel(d2, s, uv, 0) + textureSampleLevel(d2a, s, uv, l, 1u) + textureSampleLevel(dc, s, vec3f(uv, 1.0), 0);
    d += textureSampleLevel(dca, s, vec3f(uv, 1.0), l, 0i) + textureSampleLevel(d2, s, uv, 1, vec2i(1));
    c += textureSampleGrad(t2, s, uv, vec2f(0.1), vec2f(0.2)) + textureSampleGrad(t2a, s, uv, l, vec2f(0.1), vec2f(0.2), vec2i(1));
    c += textureSampleGrad(t3, s, vec3f(uv, 0.0), vec3f(0.1), vec3f(0.1)) + textureSampleGrad(tc, s, vec3f(uv, 1.0), vec3f(0.1), vec3f(0.1));
    c += textureSampleGrad(tca, s, vec3f(uv, 1.0), l, vec3f(0.1), vec3f(0.1));
    d += textureSampleCompare(d2, sc, uv, 0.5) + textureSampleCompare(d2, sc, uv, 0.5, vec2i(1)) + textureSampleCompare(d2a, sc, uv, l, 0.5);
    d += textureSampleCompare(dc, sc, vec3f(uv, 1.0), 0.5) + textureSampleCompare(dca, sc, vec3f(uv, 1.0), l, 0.5);
    d += textureSampleCompareLevel(d2, sc, uv, 0.5) + textureSampleCompareLevel(d2a, sc, uv, 1u, 0.5, vec2i(-1));
    d += textureSampleCompareLevel(dc, sc, vec3f(uv, 1.0), 0.5) + textureSampleCompareLevel(dca, sc, vec3f(uv, 1.0), l, 0.5);
    c += textureGather(0, t2, s, uv) + textureGather(1u, t2, s, uv, vec2i(1)) + textureGather(2, t2a, s, uv, l);
    c += textureGather(3, tc, s, vec3f(uv, 1.0)) + textureGather(0, tca, s, vec3f(uv, 1.0), l) + vec4f(textureGather(1, ti, s, uv));
    c += textureGather(d2, s, uv) + textureGather(d2a, s, uv, l, vec2i(1)) + textureGather(dc, s, vec3f(uv, 1.0)) + textureGather(dca, s, vec3f(uv, 1.0), l);
    c += textureGatherCompare(d2, sc, uv, 0.5) + textureGatherCompare(d2a, sc, uv, l, 0.5, vec2i(1)) + textureGatherCompare(dc, sc, vec3f(uv, 1.0), 0.5);
    c += textureGatherCompare(dca, sc, vec3f(uv, 1.0), l, 0.5);
    c += textureSampleBaseClampToEdge(t2, s, uv) + textureSampleBaseClampToEdge(ext, s, uv);
    c += vec4f(textureGather(2, tu, s, uv, 1u));
    return c + vec4f(d);
}"),
    ("textures_load_query", "@group(0) @binding(0) var t1: texture_1d<f32>;
@group(0) @binding(1) var t2: texture_2d<u32>;
@group(0) @binding(2) var t2a: texture_2d_array<i32>;
@group(0) @binding(3) var t3: texture_3d<f32>;
@group(0) @binding(4) var tm: texture_multisampled_2d<f32>;
@group(0) @binding(5) var d2: texture_depth_2d;
@group(0) @binding(6) var d2a: texture_depth_2d_array;
@group(0) @binding(7) var dm: texture_depth_multisampled_2d;
@group(0) @binding(8) var ext: texture_external;
@group(0) @binding(9) var tc: texture_cube<f32>;
@group(0) @binding(10) var tca: texture_cube_array<f32>;
@group(0) @binding(11) var s1: texture_storage_1d<rgba8unorm, write>;
@group(0) @binding(12) var s2: texture_storage_2d<r32float, read_write>;
@group(0) @binding(13) var s2a: texture_storage_2d_array<rgba16sint, read>;
@group(0) @binding(14) var s3: texture_storage_3d<rg32uint, write>;
@group(0) @binding(15) var sb: texture_storage_2d<bgra8unorm, write>;
@group(0) @binding(16) var<storage, read_write> o: array<vec4f>;
@compute @workgroup_size(4, 4)
fn main(@builtin(global_invocation_id) g: vec3u) {
    let i = vec2i(g.xy);
    var c = textureLoad(t1, i.x, 0) + vec4f(textureLoad(t2, g.xy, 1u)) + vec4f(textureLoad(t2a, i, 2u, 0));
    c += textureLoad(t3, vec3i(i, 1), 0) + textureLoad(tm, i, 3) + vec4f(textureLoad(d2, i, 0) + textureLoad(d2a, g.xy, 1, 1u));
    c += vec4f(textureLoad(dm, i, 0u)) + textureLoad(ext, g.xy) + textureLoad(s2, i) + vec4f(textureLoad(s2a, i, 1));
    textureStore(s1, i.x, c);
    textureStore(s2, g.xy, c);
    textureStore(s3, vec3u(g.xy, 0u), vec4u(1u));
    textureStore(sb, i, c);
    let n = textureDimensions(t1) + textureDimensions(t1, 1) + textureDimensions(t2).x + textureDimensions(t2a, 0u).y;
    let m = textureDimensions(t3, 1).z + textureDimensions(tm).x + textureDimensions(d2a).y + textureDimensions(dm).x;
    let q = textureDimensions(ext).x + textureDimensions(tc).x + textureDimensions(tca, 1).y + textureDimensions(s1) + textureDimensions(s3).z;
    let l = textureNumLayers(t2a) + textureNumLayers(d2a) + textureNumLayers(tca) + textureNumLayers(s2a);
    let v = textureNumLevels(t1) + textureNumLevels(t3) + textureNumLevels(d2) + textureNumLevels(tc) + textureNumLevels(tca);
    let k = textureNumSamples(tm) + textureNumSamples(dm);
    o[g.x] = c + vec4f(f32(n + m + q), f32(l), f32(v), f32(k));
    textureBarrier();
}"),
    ("stage_builtins", "struct VOut {
    @builtin(position) @invariant pos: vec4f,
    @location(0) @interpolate(linear) a: f32,
    @location(1) @interpolate(perspective, centroid) b: vec2f,
    @location(2) @interpolate(perspective, sample) c: vec3f,
    @location(3) @interpolate(flat, either) d: vec4u,
    @location(4) @interpolate(linear, sample) e: f32,
}
@vertex fn vs(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32, @location(0) p: vec4f, @location(1) q: vec2i) -> VOut {
    return VOut(p + vec4f(f32(vi + ii)), f32(q.x), vec2f(q), p.xyz, vec4u(vi), 0.5);
}
struct FOut {
    @location(0) c: vec4f,
    @location(1) i: vec2i,
    @builtin(sample_mask) m: u32,
}
@fragment fn fs(v: VOut, @builtin(front_facing) ff: bool, @builtin(sample_index) si: u32, @builtin(sample_mask) mi: u32) -> FOut {
    var o: FOut;
    o.c = v.pos + vec4f(v.a, v.b, v.c.x) + vec4f(v.d) * select(0.0, 1.0, ff) + vec4f(v.e);
    o.i = vec2i(i32(si), 0);
    o.m = mi & 3u;
    return o;
}
@fragment fn depth(@builtin(position) p: vec4f) -> @builtin(frag_depth) f32 { return p.z * 0.5; }"),
    ("clip_blend_primitive", "enable clip_distances;
enable dual_source_blending;
enable primitive_index;
struct V { @builtin(position) p: vec4f, @builtin(clip_distances) cd: array<f32, 3> }
@vertex fn vs(@builtin(vertex_index) i: u32) -> V {
    var v: V;
    v.p = vec4f(f32(i));
    v.cd[0] = 1.0;
    v.cd[2] = -1.0;
    return v;
}
struct B { @location(0) @blend_src(0) a: vec4f, @location(0) @blend_src(1) b: vec4f }
@fragment fn fs(@builtin(primitive_index) pi: u32) -> B { return B(vec4f(f32(pi)), vec4f(1.0)); }"),
    ("f16_everywhere", "enable f16;
struct U { m: mat2x2h, v: vec3h, s: f16 }
struct S { a: array<f16, 7>, h: vec4h, m: mat3x2h }
struct P { x: f16, y: vec2h }
@group(0) @binding(0) var<uniform> u: U;
@group(0) @binding(1) var<storage, read_write> st: S;
var<immediate> pc: P;
var<private> pv: vec2h = vec2h(1.0h, 2.0h);
var<workgroup> wv: array<f16, 16>;
@compute @workgroup_size(16) fn main(@builtin(local_invocation_index) i: u32) {
    wv[i] = f16(i) * 0.5h + u.s;
    workgroupBarrier();
    var x: f16 = wv[(i + 1u) % 16u] + pc.x + pv.y;
    let m = u.m * mat2x2h(st.m[0], st.m[1]);
    st.a[i % 7u] = x + m[1][0] + dot(u.v, st.h.xyz) + length(pc.y) + sqrt(abs(x)) + fma(x, x, 1.0h) + clamp(x, -1.0h, 1.0h);
    st.h = vec4h(frexp(x).fract, f16(frexp(x).exp), modf(x).whole, sin(x));
    let q = vec2f(st.h.xy) + vec2f(f32(x));
    st.m = mat3x2h(vec2h(q), pv, vec2h(f16(i32(x)), f16(u32(abs(x)))));
}
struct Io { @builtin(position) p: vec4f, @location(0) a: f16, @location(1) b: vec3h }
@vertex fn vs(@location(0) a: vec2h) -> Io { return Io(vec4f(1.0), a.x, vec3h(a, 1.0h)); }
@fragment fn fs(v: Io) -> @location(0) vec4h { return vec4h(v.b, v.a); }"),
    ("pointer_params", "struct S { a: array<vec4f, 4>, m: mat3x3f, n: u32 }
@group(0) @binding(0) var<storage, read_write> sb: S;
@group(0) @binding(1) var<uniform> ub: S;
@group(0) @binding(2) var<storage, read_write> rt: array<S>;
var<private> pv: S;
var<workgroup> wv: array<S, 2>;
fn f_storage(p: ptr<storage, vec4f, read_write>, q: ptr<uniform, vec4f>) { *p += *q; }
fn f_whole(p: ptr<storage, S, read_write>) -> u32 { (*p).n += 1u; return p.n + arrayLength(&rt); }
fn f_priv(p: ptr<private, array<vec4f, 4>>, i: u32) { p[i] = vec4f(f32(i)); }
fn f_wg(p: ptr<workgroup, S>, i: u32) -> f32 { return p.a[i].y + p.m[i][i]; }
fn f_fn(p: ptr<function, vec3f>, k: f32) { p.x = k; (*p).yz = vec2f(k, k); }
fn f_nested(p: ptr<function, array<vec3f, 3>>, i: u32) { f_fn(&(*p)[i], 2.0); f_fn(&p[2], 3.0); }
fn f_rt(p: ptr<storage, array<S>, read_write>, i: u32) -> f32 { return p[i].a[i % 4u].x; }
@compute @workgroup_size(2) fn main(@builtin(local_invocation_index) i: u32) {
    f_storage(&sb.a[i], &ub.a[i + 1u]);
    f_storage(&rt[i].a[0], &ub.a[3]);
    let n = f_whole(&sb) + f_whole(&rt[i + 5u]);
    f_priv(&pv.a, i);
    wv[i].a[i] = vec4f(1.0);
    workgroupBarrier();
    var a: array<vec3f, 3>;
    f_nested(&a, i);
    let pa = &a[1];
    f_fn(pa, 7.0);
    sb.a[0] = vec4f(a[i].x, f_wg(&wv[i], i), f_rt(&rt, i), f32(n)) + pv.a[i];
}"),
    ("control_flow", "@group(0) @binding(0) var<storage, read_write> o: array<i32>;
fn classify(x: i32) -> i32 {
    switch x {
        case 0, 1: { return 10; }
        case 2, default: { if x > 100 { break; } return 20; }
        case 3: { }
    }
    var k = 0;
    loop {
        k++;
        if k == 3 { continue; }
        if k > 9 { break; }
        continuing { k += 1; break if k > 15; }
    }
    for (var i = 0; i < 4; i++) {
        if i == x { return 30 + k; }
        switch i { case 1: { continue; } default: {} }
    }
    while k < 100 { k *= 2; if k == 64 { return -1; } }
    return k;
}
fn sc(a: bool, b: i32) -> bool { return (a && b > 0) || (!a && (b < -5 || b == 3)); }
@compute @workgroup_size(1) fn main(@builtin(global_invocation_id) g: vec3u) {
    let x = i32(g.x) - 3;
    var v = vec4i(x);
    v.yz = v.zy + vec2i(1, 2);
    v.xw += vec2i(3);
    v[g.x % 4u] = 5;
    let m = mat2x2f(1.0, 2.0, 3.0, 4.0);
    let arr = array<i32, 4>(1, 2, 3, 4);
    o[g.x] = classify(x) + select(0, 1, sc(x > 0, x)) + arr[g.x] + i32(m[g.x % 2u][g.x % 2u]) + v.x;
    if x < 0 { return; }
    o[0] += 1;
}"),
    ("overrides_everywhere", "@id(1) override a: i32 = 4;
override b: f32 = 1.5;
override c: bool;
override d = a * 3 + 1;
override e: f32 = b * b + f32(d);
override wx: u32 = 8u;
override wy = u32(a) / 2u;
var<private> p: f32 = e;
var<private> q: i32 = d;
var<workgroup> w: array<f32, wx * wy>;
var<workgroup> w2: array<u32, wx>;
@group(0) @binding(0) var<storage, read_write> o: array<f32>;
@compute @workgroup_size(wx, wy, 1)
fn main(@builtin(local_invocation_index) li: u32) {
    w[li] = p + f32(q);
    w2[li % wx] = li;
    workgroupBarrier();
    if c { o[li] = w[(li + 1u) % (wx * wy)] + f32(w2[0]); } else { o[li] = e; }
}"),
    ("atomics", "struct A { x: atomic<i32>, y: atomic<u32> }
@group(0) @binding(0) var<storage, read_write> a: A;
@group(0) @binding(1) var<storage, read_write> arr: array<atomic<u32>>;
var<workgroup> wa: atomic<i32>;
var<workgroup> wb: array<atomic<u32>, 4>;
@compute @workgroup_size(4) fn main(@builtin(local_invocation_index) i: u32) {
    let s0 = atomicAdd(&a.x, 1) + atomicSub(&a.x, 2) + atomicMax(&a.x, 3) + atomicMin(&a.x, -4);
    let s1 = atomicAnd(&a.y, 7u) | atomicOr(&a.y, 8u) | atomicXor(&a.y, 9u) | atomicExchange(&a.y, 10u);
    let r = atomicCompareExchangeWeak(&a.y, 10u, 11u);
    atomicStore(&wa, s0);
    let w = atomicLoad(&wa) + atomicAdd(&wa, 1);
    let r2 = atomicCompareExchangeWeak(&wb[i], 0u, 1u);
    atomicStore(&arr[i + 100u], s1 + select(0u, r.old_value, r.exchanged) + u32(w) + u32(r2.exchanged) + atomicLoad(&wb[3]));
}"),
    ("uniform_standard_layout", "struct U { m: mat2x2f, a: array<f32, 3>, v: vec3f }
@group(0) @binding(0) var<uniform> u: U;
@group(0) @binding(1) var<storage, read_write> o: vec4f;
@compute @workgroup_size(1) fn main() { o = vec4f(u.m[1], u.a[2], u.v.z) + vec4f(u.m * u.v.xy, 0.0, 0.0); }"),
    ("discard_fragment", "@group(0) @binding(0) var<storage, read_write> o: array<u32>;
@group(0) @binding(1) var t: texture_storage_2d<r32uint, write>;
fn maybe(x: f32) { if x < 0.5 { discard; } }
@fragment fn main(@builtin(position) p: vec4f) -> @location(0) vec4f {
    maybe(p.x);
    o[u32(p.y)] = 1u;
    let old = atomicAdd(&o2[0], 1u);
    textureStore(t, vec2u(p.xy), vec4u(old));
    return vec4f(dpdx(p.x), dpdyFine(p.y), fwidthCoarse(p.x), 1.0);
}
@group(0) @binding(2) var<storage, read_write> o2: array<atomic<u32>>;"),
    ("linear_indexing", "@group(0) @binding(0) var<storage, read_write> o: array<u32>;
@compute @workgroup_size(4, 2, 3)
fn main(@builtin(global_invocation_index) gi: u32, @builtin(workgroup_index) wi: u32, @builtin(num_workgroups) n: vec3u) {
    o[gi] = wi + n.x;
}"),
    ("values_and_conversions", "struct Inner { v: vec3f, k: i32 }
struct Outer { i: array<Inner, 2>, m: mat4x3f, b: vec2<bool> }
@group(0) @binding(0) var<storage, read_write> o: array<Inner>;
fn make(x: f32) -> Outer { return Outer(array(Inner(vec3f(x), 1), Inner(vec3f(-x), 2)), mat4x3f(), vec2(x > 0.0, true)); }
@compute @workgroup_size(1) fn main(@builtin(global_invocation_id) g: vec3u) {
    var w = make(f32(g.x));
    let k = g.x % 2u;
    w.i[k].v.y = 3.0;
    let copy = w;
    o[g.x] = copy.i[k];
    o[g.x + 1u] = Inner(vec3f(copy.m[k]), i32(copy.b[k]));
    let h = vec4i(vec4f(1e10, -1e10, 0.5, -0.5));
    let u = vec2u(vec2f(-3.0, 5e9));
    let f = vec3f(vec3i(1, 2, 3)) + vec3f(vec3u(4u)) + vec3f(vec3<bool>(true, false, true));
    let bits = bitcast<vec2u>(vec2f(1.0, 2.0)) + bitcast<vec2u>(vec2i(-1));
    o[0].k = h.x + i32(u.y) + i32(f.z) + i32(bits.x) + i32(-h.y / max(h.z, 1)) % 7;
}"),
];
