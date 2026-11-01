// vs_single: the comparison path with one draw per object -- like vs_instanced, but the
// object is pc.object_index (the push constants of the SPIR-V shader are var<immediate>:
// language extension immediate_address_space, the same 20 bytes at offset 0)

struct Object {
    pos: vec3f,
    scale: f32,
    rot: vec4f,
    color: vec3f,
    radius: f32,
}

struct Camera {
    view_proj: mat4x4f,
    planes: array<vec4f, 6>,
    light_dir: vec3f,
    ambient: f32,
}

struct CubeVertex {
    pos: vec4f,
    normal: vec4f,
}

struct Push {
    time: f32,
    count: u32,
    seed: u32,
    extent: f32,
    object_index: u32,
}

struct VsOut {
    @builtin(position) position: vec4f,
    @location(0) normal: vec3f,
    @location(1) @interpolate(flat) color: vec3f,
}

@group(0) @binding(0) var<uniform> camera: Camera;
@group(0) @binding(1) var<storage, read> objects: array<Object>;
@group(0) @binding(4) var<storage, read> cube: array<CubeVertex>;
var<immediate> pc: Push;

fn twice_sum(a: f32, b: f32, c: f32, d: f32) -> f32 {
    let s = a * b + c * d;
    return s + s;
}

fn twice_diff(a: f32, b: f32, c: f32, d: f32) -> f32 {
    let s = a * b - c * d;
    return s + s;
}

fn one_minus(a: f32, b: f32) -> f32 {
    let s = a * a + b * b;
    return 1.0 - (s + s);
}

@vertex
fn main(@builtin(vertex_index) vi: u32) -> VsOut {
    let o = objects[pc.object_index];
    let v = cube[vi];
    let q = o.rot;
    let c0 = vec3f(one_minus(q.y, q.z), twice_sum(q.x, q.y, q.w, q.z), twice_diff(q.x, q.z, q.w, q.y));
    let c1 = vec3f(twice_diff(q.x, q.y, q.w, q.z), one_minus(q.x, q.z), twice_sum(q.y, q.z, q.w, q.x));
    let c2 = vec3f(twice_sum(q.x, q.z, q.w, q.y), twice_diff(q.y, q.z, q.w, q.x), one_minus(q.x, q.y));
    let model = mat4x4f(vec4f(c0 * o.scale, 0.0), vec4f(c1 * o.scale, 0.0), vec4f(c2 * o.scale, 0.0), vec4f(o.pos, 1.0));
    let world = model * vec4f(v.pos.xyz, 1.0);
    var out: VsOut;
    out.position = camera.view_proj * world;
    out.normal = mat3x3f(c0, c1, c2) * v.normal.xyz;
    out.color = o.color;
    return out;
}
