// draw: an instance's vertices through its finished matrix (three rows; nothing per vertex beyond that),
// Lambert with one sun and an ambient light (PBR: G5c).  Everything comes from storage buffers: the
// fixed-function fetch of the same data costs three quarters more on the RTX 2070 SUPER (docs/RENDER3D.md).  A list
// starts at its first instance on Voll and at pc.first on Basis.  The material is read per fragment: small
// far objects have more vertices than fragments.

struct View {
    view_proj: mat4x4f,
    planes: array<vec4f, 6>,
    eye: vec4f,
}

// rows of the 3 x 4 matrix; a normal goes through m * (normal * normal_scale), which is the inverse
// transpose of a matrix without shear
struct Instance {
    m0: vec4f,
    m1: vec4f,
    m2: vec4f,
    normal_scale: vec3f,
    material: u32,
}

struct Lights {
    sun_dir: vec4f,
    sun_color: vec4f,
    ambient: vec4f,
}

// 64 bytes.  metallic, roughness and emissive are PBR's (G5c), the texture slots G5b's (u32::MAX: none)
struct Material {
    base_color: vec4f,
    emissive: vec3f,
    metallic: f32,
    roughness: f32,
    reserved0: f32,
    reserved1: f32,
    reserved2: f32,
    textures: vec4u,
}

// a vertex: 32 bytes in two vec4f, as the GPU likes to fetch them; the uv in their w
struct Vertex {
    position_u: vec4f,
    normal_v: vec4f,
}

struct Push {
    first: u32,
}

struct VsOut {
    @builtin(position) position: vec4f,
    @location(0) normal: vec3f,
    @location(1) @interpolate(flat) material: u32,
}

@group(0) @binding(0) var<uniform> view: View;
@group(0) @binding(1) var<storage, read> instances: array<Instance>;
@group(0) @binding(2) var<uniform> lights: Lights;
@group(0) @binding(3) var<storage, read> materials: array<Material>;
@group(0) @binding(4) var<storage, read> visible: array<u32>;
@group(0) @binding(5) var<storage, read> vertices: array<Vertex>;
var<immediate> pc: Push;

@vertex
fn vs(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> VsOut {
    let inst = instances[visible[pc.first + ii]];
    let v = vertices[vi];
    let p = vec4f(v.position_u.xyz, 1.0);
    let world = vec4f(dot(inst.m0, p), dot(inst.m1, p), dot(inst.m2, p), 1.0);
    let n = v.normal_v.xyz * inst.normal_scale;
    var out: VsOut;
    out.position = view.view_proj * world;
    out.normal = vec3f(dot(inst.m0.xyz, n), dot(inst.m1.xyz, n), dot(inst.m2.xyz, n));
    out.material = inst.material;
    return out;
}

@fragment
fn fs(f: VsOut) -> @location(0) vec4f {
    let m = materials[f.material].base_color;
    let ndl = max(dot(normalize(f.normal), lights.sun_dir.xyz), 0.0);
    return vec4f(m.xyz * (lights.ambient.xyz + lights.sun_color.xyz * ndl), m.w);
}
