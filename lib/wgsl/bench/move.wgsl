// move: object i = move_ref(i, pc) for i < pc.count (lib/shader/bench.rs, docs/SHADER.md
// "Benchmark-Vertrag").  Exact: only +, -, *, floor, conversions of integers below 2^24 and
// integer operations, in the order of the CPU reference (the back end keeps every float
// operation unfused, NoContraction).

struct Object {
    pos: vec3f,
    scale: f32,
    rot: vec4f,
    color: vec3f,
    radius: f32,
}

struct Push {
    time: f32,
    count: u32,
    seed: u32,
    extent: f32,
    object_index: u32,
}

@group(0) @binding(1) var<storage, read_write> objects: array<Object>;
var<immediate> pc: Push;

const PI = 0x1.921fb6p+1f;         // f32(pi)
const INV24 = 0x1p-24f;            // 2^-24
const INV255 = 1.0f / 255.0f;
const SQRT3 = 1.7320508f;

// PCG hash (Jarkko & Olano, "Hash Functions for GPU Rendering")
fn hash(x: u32) -> u32 {
    let s = x * 747796405u + 2891336453u;
    let w = ((s >> ((s >> 28u) + 4u)) ^ s) * 277803737u;
    return (w >> 22u) ^ w;
}

// the top 24 bits as a float in [0, 1)
fn unit(h: u32) -> f32 {
    return f32(h >> 8u) * INV24;
}

// (sin, cos) of 2 pi u: Taylor polynomials of sin / cos of pi r, r = u - floor(u + 0.5),
// then the double-angle formulas
fn sincos_turns(u: f32) -> vec2f {
    let r = u - floor(u + 0.5f);
    let x = r * PI;
    let x2 = x * x;
    var p = -1.0f / 39916800.0f;
    p = 1.0f / 362880.0f + x2 * p;
    p = -1.0f / 5040.0f + x2 * p;
    p = 1.0f / 120.0f + x2 * p;
    p = -1.0f / 6.0f + x2 * p;
    p = 1.0f + x2 * p;
    let sp = x * p;
    var q = 1.0f / 479001600.0f;
    q = -1.0f / 3628800.0f + x2 * q;
    q = 1.0f / 40320.0f + x2 * q;
    q = -1.0f / 720.0f + x2 * q;
    q = 1.0f / 24.0f + x2 * q;
    q = -0.5f + x2 * q;
    q = 1.0f + x2 * q;
    let cp = q;
    return vec2f((sp * cp) * 2.0f, cp * cp - sp * sp);
}

// 0.35 + 0.65 * byte / 255
fn channel(h: u32, shift: u32) -> f32 {
    return 0.35f + 0.65f * (f32((h >> shift) & 255u) * INV255);
}

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3u) {
    let i = gid.x;
    if i < pc.count {
        let h1 = hash(i ^ pc.seed);
        let h2 = hash(h1);
        let h3 = hash(h2);
        let h4 = hash(h3);
        let h5 = hash(h4);
        let h6 = hash(h5);
        let h7 = hash(h6);
        let h8 = hash(h7);
        let f1 = unit(h1);
        let f2 = unit(h2);
        let f3 = unit(h3);
        let f4 = unit(h4);
        let f5 = unit(h5);
        let f6 = unit(h6);
        let f7 = unit(h7);
        let orbit = pc.extent * (0.1f + 0.9f * f1);
        let height = (pc.extent * 0.25f) * (f2 - 0.5f);
        let speed = (f3 - 0.5f) * 0.2f;
        let u = f4 + speed * pc.time;
        let sc = sincos_turns(u);
        let scale = 0.25f + 0.5f * f5;
        let ua = f6 + (f6 - 0.5f) * pc.time;
        let ub = f7 + (f7 - 0.5f) * pc.time;
        let y = sincos_turns(ua);
        let x = sincos_turns(ub);
        objects[i].pos = vec3f(orbit * sc.y, height, orbit * sc.x);
        objects[i].scale = scale;
        objects[i].rot = vec4f(y.y * x.x, y.x * x.y, -(y.x * x.x), y.y * x.y);
        objects[i].color = vec3f(channel(h8, 0u), channel(h8, 8u), channel(h8, 16u));
        objects[i].radius = scale * SQRT3;
    }
}
