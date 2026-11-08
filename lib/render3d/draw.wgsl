// draw: an instance's vertices through its finished matrix (three rows; nothing per vertex beyond that),
// Lambert with one sun and an ambient light (PBR: G5c).  Everything comes from storage buffers: the
// fixed-function fetch of the same data costs three quarters more on the RTX 2070 SUPER (docs/RENDER3D.md).  A list
// starts at its first instance on Voll and at pc.first on Basis.  The material is read per fragment: small
// far objects have more vertices than fragments.  Mirrored instances run through the same code with a
// pipeline whose front faces are clockwise; their normals come out right through the signed scale.
//
// Textures (G5b): one texture array per size class (128 << k, k = 0 .. 4) and colour space in group 1, one
// repeating trilinear sampler.  A material's texture word says layer | class << 16 | mode << 20 | linear
// << 28; the array is switched here (each case samples its own binding: no binding arrays in WebGPU and
// lib/wgsl), so any slot can hold either colour space.  The
// common mode, repeating and trilinear, goes to the sampler as it is (textureSampleGrad); the others are
// done here at explicit levels, exactly since a class is a power of two: clamp and mirror by the texel
// centres of the level, nearest filters at a texel's centre, the level from the derivatives as Vulkan
// defines it (rho = the longer of the two derivatives in texels).

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

// 80 bytes: glTF's metallic-roughness material.  alpha_cutoff < 0: opaque.  Texture words (u32::MAX: none):
// base colour, metallic-roughness, normal, occlusion, then emissive.  G5b reads the base colour; the rest
// is PBR's (G5c)
struct Material {
    base_color: vec4f,
    emissive: vec3f,
    metallic: f32,
    roughness: f32,
    normal_scale: f32,
    occlusion: f32,
    alpha_cutoff: f32,
    textures: vec4u,
    emissive_texture: u32,
    reserved0: u32,
    reserved1: u32,
    reserved2: u32,
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
    @location(2) uv: vec2f,
}

@group(0) @binding(0) var<uniform> view: View;
@group(0) @binding(1) var<storage, read> instances: array<Instance>;
@group(0) @binding(2) var<uniform> lights: Lights;
@group(0) @binding(3) var<storage, read> materials: array<Material>;
@group(0) @binding(4) var<storage, read> visible: array<u32>;
@group(0) @binding(5) var<storage, read> vertices: array<Vertex>;
// sRGB (base colour, emissive) and linear (the others) arrays of 128, 256, 512, 1024, 2048 texels
@group(1) @binding(0) var s0: texture_2d_array<f32>;
@group(1) @binding(1) var s1: texture_2d_array<f32>;
@group(1) @binding(2) var s2: texture_2d_array<f32>;
@group(1) @binding(3) var s3: texture_2d_array<f32>;
@group(1) @binding(4) var s4: texture_2d_array<f32>;
@group(1) @binding(5) var l0: texture_2d_array<f32>;
@group(1) @binding(6) var l1: texture_2d_array<f32>;
@group(1) @binding(7) var l2: texture_2d_array<f32>;
@group(1) @binding(8) var l3: texture_2d_array<f32>;
@group(1) @binding(9) var l4: texture_2d_array<f32>;
@group(1) @binding(10) var smp: sampler;
var<immediate> pc: Push;

const NO_TEXTURE: u32 = 0xffffffffu;
// the mode byte of repeating, trilinear sampling: the sampler's own
const TRILINEAR_REPEAT: u32 = 128u;

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
    out.uv = vec2f(v.position_u.w, v.normal_v.w);
    return out;
}

// one array (0 .. 4 sRGB, 5 .. 9 linear) at an explicit level (bilinear inside it)
fn array_level(which: u32, uv: vec2f, layer: u32, level: f32) -> vec4f {
    switch which {
        case 0u: { return textureSampleLevel(s0, smp, uv, layer, level); }
        case 1u: { return textureSampleLevel(s1, smp, uv, layer, level); }
        case 2u: { return textureSampleLevel(s2, smp, uv, layer, level); }
        case 3u: { return textureSampleLevel(s3, smp, uv, layer, level); }
        case 4u: { return textureSampleLevel(s4, smp, uv, layer, level); }
        case 5u: { return textureSampleLevel(l0, smp, uv, layer, level); }
        case 6u: { return textureSampleLevel(l1, smp, uv, layer, level); }
        case 7u: { return textureSampleLevel(l2, smp, uv, layer, level); }
        case 8u: { return textureSampleLevel(l3, smp, uv, layer, level); }
        default: { return textureSampleLevel(l4, smp, uv, layer, level); }
    }
}

// one array, trilinear by the sampler from the derivatives
fn array_grad(which: u32, uv: vec2f, layer: u32, dx: vec2f, dy: vec2f) -> vec4f {
    switch which {
        case 0u: { return textureSampleGrad(s0, smp, uv, layer, dx, dy); }
        case 1u: { return textureSampleGrad(s1, smp, uv, layer, dx, dy); }
        case 2u: { return textureSampleGrad(s2, smp, uv, layer, dx, dy); }
        case 3u: { return textureSampleGrad(s3, smp, uv, layer, dx, dy); }
        case 4u: { return textureSampleGrad(s4, smp, uv, layer, dx, dy); }
        case 5u: { return textureSampleGrad(l0, smp, uv, layer, dx, dy); }
        case 6u: { return textureSampleGrad(l1, smp, uv, layer, dx, dy); }
        case 7u: { return textureSampleGrad(l2, smp, uv, layer, dx, dy); }
        case 8u: { return textureSampleGrad(l3, smp, uv, layer, dx, dy); }
        default: { return textureSampleGrad(l4, smp, uv, layer, dx, dy); }
    }
}

// a coordinate on a level of n texels for the repeating sampler: wrap 0 repeat, 1 clamp to edge, 2 mirrored
// repeat (clamped on the mirrored coordinate: at the seam both are the edge texel); nearest: a texel's centre
fn wrap_coord(u: f32, wrap: u32, n: f32, nearest: bool) -> f32 {
    var x = u;
    if wrap == 2u {
        let t = x - 2.0 * floor(x * 0.5);
        x = select(t, 2.0 - t, t > 1.0);
    }
    if nearest {
        var i = floor(x * n);
        if wrap == 0u {
            i = i - n * floor(i / n);
        } else {
            i = clamp(i, 0.0, n - 1.0);
        }
        return (i + 0.5) / n;
    }
    if wrap != 0u {
        x = clamp(x, 0.5 / n, 1.0 - 0.5 / n);
    }
    return x;
}

fn array_at(which: u32, layer: u32, uv: vec2f, wrap: vec2u, size: f32, level: f32, nearest: bool) -> vec4f {
    let n = size / exp2(level);
    return array_level(which, vec2f(wrap_coord(uv.x, wrap.x, n, nearest), wrap_coord(uv.y, wrap.y, n, nearest)), layer, level);
}

// a texture word's colour at uv (the derivatives of uv come from uniform control flow), sRGB decoded
fn sample_texture(t: u32, uv: vec2f, dx: vec2f, dy: vec2f) -> vec4f {
    let layer = t & 0xffffu;
    let cls = (t >> 16u) & 15u;
    let mode = (t >> 20u) & 255u;
    let which = cls + 5u * ((t >> 28u) & 1u);
    if mode == TRILINEAR_REPEAT {
        return array_grad(which, uv, layer, dx, dy);
    }
    let size = f32(128u << cls);
    let wrap = vec2u(mode & 3u, (mode >> 2u) & 3u);
    let lod = log2(max(length(dx * size), length(dy * size)));
    if !(lod > 0.0) {
        // magnified: the mag filter on level 0
        return array_at(which, layer, uv, wrap, size, 0.0, (mode & 16u) != 0u);
    }
    let nearest = (mode & 32u) != 0u;
    let mip = mode >> 6u;
    let top = f32(7u + cls);
    if mip == 0u {
        return array_at(which, layer, uv, wrap, size, 0.0, nearest);
    }
    if mip == 1u {
        return array_at(which, layer, uv, wrap, size, min(floor(lod + 0.5), top), nearest);
    }
    let d = min(lod, top);
    let l0 = floor(d);
    let a = array_at(which, layer, uv, wrap, size, l0, nearest);
    if l0 >= top {
        return a;
    }
    return mix(a, array_at(which, layer, uv, wrap, size, l0 + 1.0, nearest), d - l0);
}

@fragment
fn fs(f: VsOut) -> @location(0) vec4f {
    let dx = dpdxFine(f.uv);
    let dy = dpdyFine(f.uv);
    let m = materials[f.material];
    var c = m.base_color;
    let t = m.textures.x;
    if t != NO_TEXTURE {
        c = c * sample_texture(t, f.uv, dx, dy);
    }
    if c.w < m.alpha_cutoff {
        discard;
    }
    let ndl = max(dot(normalize(f.normal), lights.sun_dir.xyz), 0.0);
    return vec4f(c.xyz * (lights.ambient.xyz + lights.sun_color.xyz * ndl), 1.0);
}
