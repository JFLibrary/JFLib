// draw: an instance's vertices through its finished matrix (three rows; nothing per vertex beyond that),
// shaded after glTF 2.0's metallic-roughness model (G5c; docs/RENDER3D.md "PBR").  Everything comes from
// storage buffers: the fixed-function fetch of the same data costs three quarters more on the RTX 2070
// SUPER (docs/RENDER3D.md).  A list starts at its first instance on Voll and at pc.first on Basis.  The
// material is read per fragment: small far objects have more vertices than fragments.  Mirrored instances
// run through the same code with a pipeline whose front faces are clockwise; their normals come out right
// through the signed scale.  Lists per alpha class: `fs` draws opaque lists and has no discard (the early depth test
// stays the pipeline's), `fs_mask` MASK lists; both shade alike (`shade`).
//
// Textures (G5b): one texture array per size class (128 << k, k = 0 .. 4) and colour space in group 1, one
// repeating trilinear sampler.  A material's texture word says layer | class << 16 | mode << 20 | linear
// << 28; the array is switched here (each case samples its own binding: no binding arrays in WebGPU and
// lib/wgsl), so any slot can hold either colour space.  The
// common mode, repeating and trilinear, goes to the sampler as it is (textureSampleGrad); the others are
// done here at explicit levels, exactly since a class is a power of two: clamp and mirror by the texel
// centres of the level, nearest filters at a texel's centre, the level from the derivatives as Vulkan
// defines it (rho = the longer of the two derivatives in texels).
//
// Shading (G5c): the BRDF of glTF 2.0 Appendix B (Lambert diffuse, GGX / Trowbridge-Reitz, the separable
// Smith visibility, Schlick's Fresnel), linear light in the units of KHR_lights_punctual (the sun in lux),
// times the scene's exposure, then Khronos PBR Neutral (or no tone mapping) right here: no pass after it.
// Normal maps without a TANGENT attribute: the triangle's tangent frame from the derivatives of the world
// position and the uv (bumped; how that differs from MikkTSpace: docs/RENDER3D.md).  A uniform ambient
// light reaches the surface through the split-sum approximation with its second sum from a table (env_ab,
// dfg.rs); occlusion acts on it only.
//
// Shadows (G5c): the sun's cascades are views of their own (cull.wgsl fills their lists), drawn into the layers
// of a depth array by vs_depth (no fragment shader: opaque lists, and MASK lists without their holes) or vs_shadow +
// fs_shadow (MASK lists' holes), both with a cascade's orthographic view_proj in `view`; the main pass filters them
// with a comparison sampler (sun_shadow).
//
// sRGB (G5e2): an 8-bit target without sRGB (Rgba8Unorm, Bgra8Unorm: a window's swapchain, a view of lib/ui) would
// show the linear light too dark, so the main pass's fragment shaders encode it here when the pipeline sets
// ENCODE_SRGB (RendererDesc::encode_srgb); an sRGB target's hardware encodes by itself, a float target stays linear.

// (G5e2) encode the colour to sRGB before it goes to the target (lib.rs sets it for 8-bit targets without sRGB)
@id(0) override ENCODE_SRGB: bool = false;

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

// a shadow cascade (G5c): rows taking a point (relative to the origin) to (u, v, depth) of its map; info.x
// one texel in world units, info.y the depth of one world unit
struct Cascade {
    u: vec4f,
    v: vec4f,
    d: vec4f,
    info: vec4f,
}

// sun_dir: towards the sun (a unit vector), w the exposure; sun_color: illuminance per channel (lux), w 1
// for the neutral tone mapper, 0 for none; ambient: the radiance of a uniform environment; shadow.x the
// sun's cascades (0: no shadows), shadow.y texels a side of each cascade's map, shadow.z the point and spot
// lights
// env.x the environment's strength (0: Sun::ambient instead), env.y its cube's top level; sh the irradiance's
// 9 harmonics divided by pi
struct Lights {
    sun_dir: vec4f,
    sun_color: vec4f,
    ambient: vec4f,
    shadow: vec4f,
    cascades: array<Cascade, 4>,
    env: vec4f,
    sh: array<vec4f, 9>,
}

// 80 bytes: glTF's metallic-roughness material.  alpha_cutoff < 0: opaque.  Texture words (u32::MAX: none):
// base colour, metallic-roughness, normal, occlusion, then emissive
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
    // relative to the scene's origin, as the eye
    @location(3) world: vec3f,
}

@group(0) @binding(0) var<uniform> view: View;
@group(0) @binding(1) var<storage, read> instances: array<Instance>;
@group(0) @binding(2) var<uniform> lights: Lights;
@group(0) @binding(3) var<storage, read> materials: array<Material>;
@group(0) @binding(4) var<storage, read> visible: array<u32>;
@group(0) @binding(5) var<storage, read> vertices: array<Vertex>;
// (G5c) a point or spot light (KHR_lights_punctual): position relative to the origin and range (0: none);
// colour times intensity (candela) and the kind (1: spot); a spot's axis; its cone as scale and offset of cos
struct Punctual {
    pos_range: vec4f,
    color_kind: vec4f,
    dir: vec4f,
    cone: vec4f,
}
@group(0) @binding(6) var<storage, read> punctual: array<Punctual>;
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
// (G5c) the split-sum's second sum (A, B) over n.v (x) and roughness (y), 32 x 32 (dfg.rs)
@group(1) @binding(11) var<storage, read> dfg: array<vec2f>;
// (G5c) group 2, the main pass's alone (the shadow passes write the map: WebGPU forbids it bound there):
// the sun's cascades in the layers of one depth array (reverse Z), compared by a linear comparison sampler
@group(2) @binding(0) var shadow_map: texture_depth_2d_array;
@group(2) @binding(1) var shadow_cmp: sampler_comparison;
// (G5c) the environment's cube, level k prefiltered for roughness k / top (env.rs)
@group(2) @binding(2) var env_cube: texture_cube<f32>;
@group(2) @binding(3) var env_smp: sampler;
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
    out.world = world.xyz;
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

// one array's texel at a level
fn array_load(which: u32, c: vec2i, layer: u32, level: i32) -> vec4f {
    switch which {
        case 0u: { return textureLoad(s0, c, layer, level); }
        case 1u: { return textureLoad(s1, c, layer, level); }
        case 2u: { return textureLoad(s2, c, layer, level); }
        case 3u: { return textureLoad(s3, c, layer, level); }
        case 4u: { return textureLoad(s4, c, layer, level); }
        case 5u: { return textureLoad(l0, c, layer, level); }
        case 6u: { return textureLoad(l1, c, layer, level); }
        case 7u: { return textureLoad(l2, c, layer, level); }
        case 8u: { return textureLoad(l3, c, layer, level); }
        default: { return textureLoad(l4, c, layer, level); }
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

// mirrored repeat (2) as a coordinate in 0 ..= 1: at the seam both sides are the edge texel
fn mirror(u: f32) -> f32 {
    let t = u - 2.0 * floor(u * 0.5);
    return select(t, 2.0 - t, t > 1.0);
}

// a coordinate on a level of n texels for the repeating sampler: wrap 0 repeat, 1 clamp to edge, 2 mirrored
// repeat (clamped on the mirrored coordinate)
fn wrap_coord(u: f32, wrap: u32, n: f32) -> f32 {
    var x = u;
    if wrap == 2u {
        x = mirror(x);
    }
    if wrap != 0u {
        x = clamp(x, 0.5 / n, 1.0 - 0.5 / n);
    }
    return x;
}

// the texel of a coordinate on a level of n texels (nearest filtering), wrapped as wrap_coord does
fn wrap_texel(u: f32, wrap: u32, n: f32) -> i32 {
    var x = u;
    if wrap == 2u {
        x = mirror(x);
    }
    var i = floor(x * n);
    if wrap == 0u {
        i = i - n * floor(i / n);
    } else {
        i = clamp(i, 0.0, n - 1.0);
    }
    return i32(i);
}

// a level's colour at uv: nearest filtering loads the texel (exact: a texel's centre through the linear
// sampler mixes in 1/256 of a neighbour on this hardware, which shading can multiply), else bilinear
fn array_at(which: u32, layer: u32, uv: vec2f, wrap: vec2u, size: f32, level: f32, nearest: bool) -> vec4f {
    let n = size / exp2(level);
    if nearest {
        return array_load(which, vec2i(wrap_texel(uv.x, wrap.x, n), wrap_texel(uv.y, wrap.y, n)), layer, i32(level));
    }
    return array_level(which, vec2f(wrap_coord(uv.x, wrap.x, n), wrap_coord(uv.y, wrap.y, n)), layer, level);
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

// ---------------------------------------------------------------- shadow passes (G5c)

struct ShadowOut {
    @builtin(position) position: vec4f,
    @location(0) @interpolate(flat) material: u32,
    @location(1) uv: vec2f,
}

// a vertex of a cascade (its view_proj, orthographic): casters nearer the sun than the cascade's near plane
// are flattened onto it (pancaking), so they cast without a depth range reaching the sun
fn shadow_position(vi: u32, ii: u32) -> vec4f {
    let inst = instances[visible[pc.first + ii]];
    let v = vertices[vi];
    let p = vec4f(v.position_u.xyz, 1.0);
    let world = vec4f(dot(inst.m0, p), dot(inst.m1, p), dot(inst.m2, p), 1.0);
    var out = view.view_proj * world;
    out.z = min(out.z, out.w);
    return out;
}

// depth only: opaque casters (and MASK casters with full shadows)
@vertex
fn vs_depth(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> @builtin(position) vec4f {
    return shadow_position(vi, ii);
}

@vertex
fn vs_shadow(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> ShadowOut {
    var out: ShadowOut;
    out.position = shadow_position(vi, ii);
    let inst = instances[visible[pc.first + ii]];
    let v = vertices[vi];
    out.material = inst.material;
    out.uv = vec2f(v.position_u.w, v.normal_v.w);
    return out;
}

// MASK's holes let the sun through (MASK lists only; an opaque material would return at once)
@fragment
fn fs_shadow(f: ShadowOut) {
    let dx = dpdxFine(f.uv);
    let dy = dpdyFine(f.uv);
    let m = materials[f.material];
    if m.alpha_cutoff < 0.0 {
        return;
    }
    var a = m.base_color.w;
    if m.textures.x != NO_TEXTURE {
        a = a * sample_texture(m.textures.x, f.uv, dx, dy).w;
    }
    if a < m.alpha_cutoff {
        discard;
    }
}

// ---------------------------------------------------------------- shading

// receivers look up their cascade a little off the surface: along the normal by NORMAL_BIAS texels of the
// cascade (enough for the filter's reach on any slope) and towards the sun by DEPTH_BIAS texels
const NORMAL_BIAS: f32 = 2.0;
const DEPTH_BIAS: f32 = 0.25;

// how much of the sun reaches a point (relative to the origin) with geometric normal ng: the first cascade
// whose map holds it with the filter's reach, 3 x 3 taps of the bilinear comparison (a 4 x 4 texel tent);
// 1 outside all of them
fn sun_shadow(p: vec3f, ng: vec3f) -> f32 {
    let n = u32(lights.shadow.x);
    let size = lights.shadow.y;
    let texel = 1.0 / size;
    for (var k = 0u; k < n; k++) {
        let c = lights.cascades[k];
        let q = vec4f(p + ng * (c.info.x * NORMAL_BIAS), 1.0);
        let s = vec3f(dot(c.u, q), dot(c.v, q), dot(c.d, q));
        let lo = 2.0 * texel;
        if s.x >= lo && s.x <= 1.0 - lo && s.y >= lo && s.y <= 1.0 - lo && s.z >= 0.0 && s.z <= 1.0 {
            let r = s.z + c.info.x * c.info.y * DEPTH_BIAS;
            var sum = 0.0;
            for (var j = -1; j <= 1; j++) {
                for (var i = -1; i <= 1; i++) {
                    sum += textureSampleCompareLevel(shadow_map, shadow_cmp, s.xy + vec2f(f32(i), f32(j)) * texel, k, r);
                }
            }
            return sum * (1.0 / 9.0);
        }
    }
    return 1.0;
}

const PI: f32 = 3.141592653589793;
// the least alpha (= roughness squared) the BRDF takes: roughness 0 would divide 0 by 0 at the highlight
const ALPHA_MIN: f32 = 0.001;

// glTF 2.0 Appendix B for one light from l: f = (1 - F) c_diff / pi + F D V with Schlick's F, GGX's D and the
// separable Smith V = G / (4 |N.L| |N.V|); times N.L.  a2 = alpha squared
fn direct(n: vec3f, v: vec3f, l: vec3f, c_diff: vec3f, f0: vec3f, a2: f32) -> vec3f {
    let nl = dot(n, l);
    if !(nl > 0.0) {
        return vec3f(0.0);
    }
    let nv = clamp(dot(n, v), 0.0, 1.0);
    let hv = l + v;
    let h = hv * inverseSqrt(max(dot(hv, hv), 1e-30));
    let nh = clamp(dot(n, h), 0.0, 1.0);
    let vh = clamp(dot(v, h), 0.0, 1.0);
    let x = 1.0 - vh;
    let x2 = x * x;
    let f = f0 + (1.0 - f0) * (x2 * x2 * x);
    let t = nh * nh * (a2 - 1.0) + 1.0;
    let d = a2 / (PI * t * t);
    let vis = 1.0 / ((nl + sqrt(a2 + (1.0 - a2) * nl * nl)) * (nv + sqrt(a2 + (1.0 - a2) * nv * nv)));
    return ((1.0 - f) * c_diff * (1.0 / PI) + f * (d * vis)) * nl;
}

// a point or spot light's light at p (KHR_lights_punctual): 1 / d^2, its range's window clamp(1 - (d / range)^4,
// 0, 1), a spot's smooth cone squared; no shadows
fn punctual_light(q: Punctual, p: vec3f, n: vec3f, v: vec3f, c_diff: vec3f, f0: vec3f, a2: f32) -> vec3f {
    let d = q.pos_range.xyz - p;
    let dist2 = dot(d, d);
    if !(dist2 > 0.0) {
        return vec3f(0.0);
    }
    var att = 1.0 / dist2;
    let range = q.pos_range.w;
    if range > 0.0 {
        let r2 = dist2 / (range * range);
        att = att * clamp(1.0 - r2 * r2, 0.0, 1.0);
    }
    let l = d * inverseSqrt(dist2);
    if q.color_kind.w != 0.0 {
        let t = clamp(dot(q.dir.xyz, -l) * q.cone.x + q.cone.y, 0.0, 1.0);
        att = att * t * t;
    }
    if att == 0.0 {
        return vec3f(0.0);
    }
    return direct(n, v, l, c_diff, f0, a2) * q.color_kind.xyz * att;
}

// the second sum of the split-sum approximation: scale and bias of f0, bilinear in the table (dfg.rs)
fn env_ab(roughness: f32, nv: f32) -> vec2f {
    let x = clamp(nv, 0.0, 1.0) * 31.0;
    let y = clamp(roughness, 0.0, 1.0) * 31.0;
    let i = min(u32(x), 30u);
    let j = min(u32(y), 30u);
    let fx = x - f32(i);
    let fy = y - f32(j);
    let k = j * 32u + i;
    return mix(mix(dfg[k], dfg[k + 1u], fx), mix(dfg[k + 32u], dfg[k + 33u], fx), fy);
}

// the environment's irradiance over pi at a unit normal: its 9 harmonics (a polynomial, no texture)
fn sh_irradiance(n: vec3f) -> vec3f {
    let c = lights.sh;
    return c[0].xyz * 0.282095 + c[1].xyz * (0.488603 * n.y) + c[2].xyz * (0.488603 * n.z) + c[3].xyz * (0.488603 * n.x)
        + c[4].xyz * (1.092548 * n.x * n.y) + c[5].xyz * (1.092548 * n.y * n.z) + c[6].xyz * (0.315392 * (3.0 * n.z * n.z - 1.0))
        + c[7].xyz * (1.092548 * n.x * n.z) + c[8].xyz * (0.546274 * (n.x * n.x - n.y * n.y));
}

// Khronos PBR Neutral: base colours stay as they are below 0.76, highlights are compressed towards white
fn neutral(c: vec3f) -> vec3f {
    let x = min(c.x, min(c.y, c.z));
    let offset = select(0.04, x - 6.25 * x * x, x < 0.08);
    var col = c - offset;
    let peak = max(col.x, max(col.y, col.z));
    if peak < 0.76 {
        return col;
    }
    let d = 0.24;
    let new_peak = 1.0 - d * d / (peak + d - 0.76);
    col = col * (new_peak / peak);
    let g = 1.0 - 1.0 / (0.15 * (peak - new_peak) + 1.0);
    return mix(col, vec3f(new_peak), g);
}

// a normal map's tangent-space normal t on the geometric normal ng: the tangent is the triangle's dp/du
// (from the derivatives of the world position p and of uv), made orthogonal to ng; the bitangent
// cross(ng, tangent) points where v decreases (glTF: +y is up in the image), so mirrored uv and mirrored
// instances come out right.  No uv gradient (uv constant): ng
fn bumped(ng: vec3f, t: vec3f, px: vec3f, py: vec3f, dx: vec2f, dy: vec2f) -> vec3f {
    let det = dx.x * dy.y - dx.y * dy.x;
    if det == 0.0 {
        return ng;
    }
    let s = sign(det);
    let dpdu = (px * dy.y - py * dx.y) * s;
    let dpdv = (py * dx.x - px * dy.x) * s;
    let tt = dpdu - ng * dot(ng, dpdu);
    let lt = dot(tt, tt);
    if !(lt > 0.0) {
        return ng;
    }
    let tn = tt * inverseSqrt(lt);
    var b = cross(ng, tn);
    if dot(b, dpdv) > 0.0 {
        b = -b;
    }
    return normalize(tn * t.x + b * t.y + ng * t.z);
}

// the base colour times its texture (alpha: what MASK compares with its cutoff)
fn base_color(m: Material, uv: vec2f, dx: vec2f, dy: vec2f) -> vec4f {
    var c = m.base_color;
    if m.textures.x != NO_TEXTURE {
        c = c * sample_texture(m.textures.x, uv, dx, dy);
    }
    return c;
}

// everything after the base colour: the other textures, glTF's BRDF for the sun (its shadow), the point and spot
// lights, the ambient light or the environment, emission, exposure, the tone mapper.  dx, dy: the derivatives of uv,
// px, py: those of the world position (taken in uniform control flow by the caller)
fn shade(m: Material, c: vec4f, normal: vec3f, uv: vec2f, world: vec3f, dx: vec2f, dy: vec2f, px: vec3f, py: vec3f) -> vec4f {
    var metallic = m.metallic;
    var roughness = m.roughness;
    if m.textures.y != NO_TEXTURE {
        let mr = sample_texture(m.textures.y, uv, dx, dy);
        roughness = roughness * mr.y;
        metallic = metallic * mr.z;
    }
    metallic = clamp(metallic, 0.0, 1.0);
    roughness = clamp(roughness, 0.0, 1.0);
    let ng = normalize(normal);
    var n = ng;
    if m.textures.z != NO_TEXTURE {
        let t = sample_texture(m.textures.z, uv, dx, dy).xyz * 2.0 - 1.0;
        n = bumped(ng, t * vec3f(m.normal_scale, m.normal_scale, 1.0), px, py, dx, dy);
    }
    var ao = 1.0;
    if m.textures.w != NO_TEXTURE {
        ao = 1.0 + m.occlusion * (sample_texture(m.textures.w, uv, dx, dy).x - 1.0);
    }
    var emissive = m.emissive;
    if m.emissive_texture != NO_TEXTURE {
        emissive = emissive * sample_texture(m.emissive_texture, uv, dx, dy).xyz;
    }
    let v = normalize(view.eye.xyz - world);
    let c_diff = c.xyz * (1.0 - metallic);
    let f0 = mix(vec3f(0.04), c.xyz, metallic);
    let a = max(roughness * roughness, ALPHA_MIN);
    var light = direct(n, v, lights.sun_dir.xyz, c_diff, f0, a * a) * lights.sun_color.xyz;
    if lights.shadow.x > 0.0 && any(light > vec3f(0.0)) {
        light = light * sun_shadow(world, ng);
    }
    let count = u32(lights.shadow.z);
    for (var i = 0u; i < count; i++) {
        light = light + punctual_light(punctual[i], world, n, v, c_diff, f0, a * a);
    }
    // the uniform environment: what the specular lobe does not take goes to the diffuse
    let ab = env_ab(roughness, clamp(dot(n, v), 0.0, 1.0));
    let spec = f0 * ab.x + ab.y;
    // (G5c) or the environment: its harmonics for the diffuse part, its prefiltered cube along the reflection
    var diffuse = lights.ambient.xyz;
    var specular = lights.ambient.xyz;
    if lights.env.x > 0.0 {
        diffuse = sh_irradiance(n) * lights.env.x;
        specular = textureSampleLevel(env_cube, env_smp, reflect(-v, n), roughness * lights.env.y).xyz * lights.env.x;
    }
    light = light + (specular * spec + diffuse * c_diff * (1.0 - spec)) * ao + emissive;
    var out = light * lights.sun_dir.w;
    if lights.sun_color.w != 0.0 {
        out = neutral(out);
    }
    if ENCODE_SRGB {
        out = srgb_encode(out);
    }
    return vec4f(out, 1.0);
}

// (G5e2) linear light to sRGB's code values by the definition (IEC 61966-2-1, not a gamma of 2.2), after clamping to
// 0 ..= 1 as the target would
fn srgb_encode(c: vec3f) -> vec3f {
    let v = clamp(c, vec3f(0.0), vec3f(1.0));
    return select(1.055 * pow(v, vec3f(1.0 / 2.4)) - 0.055, v * 12.92, v <= vec3f(0.0031308));
}

// opaque lists: nothing is dropped, no discard anywhere (alpha_cutoff is not read)
@fragment
fn fs(f: VsOut) -> @location(0) vec4f {
    // (derivatives in uniform control flow, before anything depends on the material)
    let dx = dpdxFine(f.uv);
    let dy = dpdyFine(f.uv);
    let px = dpdxFine(f.world);
    let py = dpdyFine(f.world);
    let m = materials[f.material];
    return shade(m, base_color(m, f.uv, dx, dy), f.normal, f.uv, f.world, dx, dy, px, py);
}

// MASK lists: fragments whose alpha is below the cutoff are dropped
@fragment
fn fs_mask(f: VsOut) -> @location(0) vec4f {
    let dx = dpdxFine(f.uv);
    let dy = dpdyFine(f.uv);
    let px = dpdxFine(f.world);
    let py = dpdyFine(f.world);
    let m = materials[f.material];
    let c = base_color(m, f.uv, dx, dy);
    if c.w < m.alpha_cutoff {
        discard;
    }
    return shade(m, c, f.normal, f.uv, f.world, dx, dy, px, py);
}
