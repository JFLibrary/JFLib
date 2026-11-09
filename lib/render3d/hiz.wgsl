// hiz: the depth pyramid of a frame for the camera's occlusion test (G5d; docs/RENDER3D.md "Verdeckung").  Level k
// (k >= 1) holds the smallest depth (reverse Z: the farthest) of the pixels [x 2^k, (x + 1) 2^k) x [y 2^k,
// (y + 1) 2^k), cut at the image's edge: ceil(w / 2^k) x ceil(h / 2^k) texels in mip k - 1 of a square R32Float
// texture of 2^(levels - 1) texels (the mips of a texture halve with rounding down; what lies beyond the image is
// never read).  from_depth makes level 1 from the depth buffer, down each further level from the one before; a texel
// takes the 2 x 2 below it, those beyond the edge (odd sizes) are the edge's again.

struct Push {
    // the depth buffer's size, the level made
    w: u32,
    h: u32,
    level: u32,
}

@group(0) @binding(0) var depth: texture_depth_2d;
@group(0) @binding(1) var dst: texture_storage_2d<r32float, write>;
@group(0) @binding(2) var src: texture_2d<f32>;
var<immediate> pc: Push;

// texels of level k that cover the image (level 0: the depth buffer)
fn valid(k: u32) -> vec2i {
    let a = (1u << k) - 1u;
    return vec2i(vec2u((pc.w + a) >> k, (pc.h + a) >> k));
}

@compute @workgroup_size(8, 8)
fn from_depth(@builtin(global_invocation_id) id: vec3u) {
    let t = vec2i(id.xy);
    if any(t >= valid(1u)) {
        return;
    }
    let last = valid(0u) - 1;
    let p = t * 2;
    let q = min(p + 1, last);
    let m = min(min(textureLoad(depth, p, 0), textureLoad(depth, vec2i(q.x, p.y), 0)), min(textureLoad(depth, vec2i(p.x, q.y), 0), textureLoad(depth, q, 0)));
    textureStore(dst, t, vec4f(m, 0.0, 0.0, 1.0));
}

@compute @workgroup_size(8, 8)
fn down(@builtin(global_invocation_id) id: vec3u) {
    let t = vec2i(id.xy);
    if any(t >= valid(pc.level)) {
        return;
    }
    let last = valid(pc.level - 1u) - 1;
    let p = t * 2;
    let q = min(p + 1, last);
    let m = min(min(textureLoad(src, p, 0).x, textureLoad(src, vec2i(q.x, p.y), 0).x), min(textureLoad(src, vec2i(p.x, q.y), 0).x, textureLoad(src, q, 0).x));
    textureStore(dst, t, vec4f(m, 0.0, 0.0, 1.0));
}
