// post: (G7f) passes over the finished image (docs/RENDER3D.md "Umrisse", "Kantenglättung"), each one triangle over the
// whole target.
//
// fs_outline: lines on the edges of the depth buffer.  Reverse Z's depth (1 / z, or linear in z for an orthographic view)
// is linear in the screen on every plane, so its second difference across a pixel is 0 on planes, whatever their slope,
// and not 0 where the surface bends (a crease) or jumps (a silhouette).  Divided by what one pixel's width of depth along
// a surface at 45 degrees is there, it is the change of the surface's slope (tan of its angle to the view) across the
// pixel.  Above `crease` it is a line, half of it on either side of the crease; a jump (above JUMP) is a line on its near
// side only.  `width` pixels apart, so lines are `width` pixels wide on a silhouette's near side.  With a viewmodel its
// pixels (depth above 0.5) take its lens.  The colour is laid over the image in premultiplied alpha.
//
// fs_fxaa: FXAA 3.11 (Timothy Lottes, NVIDIA), its quality preset 12 (five steps along an edge: 1, 1.5, 2, 4, 12 pixels),
// subpixel 0.75, edge threshold 0.166 (at least 0.0833): the scene drawn into an image of its own, read here with a
// bilinear sampler and written to the target.  Luma from the image's values; LINEAR (sRGB and float targets, whose
// sampler returns linear light): the square root of it.  The steps are unrolled (no array indexed at run time).

@id(0) override ENCODE_SRGB: bool = false;
@id(1) override LINEAR: bool = false;

// colour (linear) and opacity; width (pixels), crease, whether a viewmodel takes the depths above 0.5; the world's lens:
// zz, zw (depth = zz + zw / z, orthographic zz + zw z), fy (sy), orthographic; the viewmodel's the same
struct Outline {
    color: vec4f,
    params: vec4f,
    world: vec4f,
    vm: vec4f,
}

@group(0) @binding(0) var<uniform> outline: Outline;
@group(0) @binding(1) var depth: texture_depth_2d;
@group(0) @binding(2) var image: texture_2d<f32>;
@group(0) @binding(3) var smp: sampler;

// a slope change above this is a jump: a line on its near side only
const JUMP: f32 = 16.0;

@vertex
fn vs_full(@builtin(vertex_index) vi: u32) -> @builtin(position) vec4f {
    let x = f32((vi << 1u) & 2u);
    let y = f32(vi & 2u);
    return vec4f(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
}

fn depth_at(p: vec2i, top: vec2i) -> f32 {
    return textureLoad(depth, clamp(p, vec2i(0), top), 0);
}

// (draw.wgsl's) linear light to sRGB's code values by the definition
fn srgb_encode(c: vec3f) -> vec3f {
    let v = clamp(c, vec3f(0.0), vec3f(1.0));
    return select(1.055 * pow(v, vec3f(1.0 / 2.4)) - 0.055, v * 12.92, v <= vec3f(0.0031308));
}

@fragment
fn fs_outline(@builtin(position) at: vec4f) -> @location(0) vec4f {
    let size = vec2i(textureDimensions(depth));
    let top = size - 1;
    let p = vec2i(at.xy);
    let dc = depth_at(p, top);
    // (nothing drawn: cleared, infinitely far)
    if !(dc > 0.0) {
        return vec4f(0.0);
    }
    let w = i32(outline.params.x);
    let l = depth_at(p - vec2i(w, 0), top);
    let r = depth_at(p + vec2i(w, 0), top);
    let u = depth_at(p - vec2i(0, w), top);
    let d = depth_at(p + vec2i(0, w), top);
    var k = outline.world;
    if outline.params.z != 0.0 && dc > 0.5 {
        k = outline.vm;
    }
    // one pixel's angle (perspective) or width (orthographic), and the depth one pixel's width along a 45 degree surface
    // makes there, times the pixels apart
    let px = 2.0 / (k.z * f32(size.y));
    var s = (dc - k.x) * px;
    if k.w != 0.0 {
        s = abs(k.y) * px;
    }
    s = s * f32(w);
    let lx = (l + r - 2.0 * dc) / s;
    let ly = (u + d - 2.0 * dc) / s;
    let e = max(abs(lx), abs(ly));
    let t = outline.params.y;
    var cover = smoothstep(t, t * 1.5 + 1e-3, e);
    // a jump: only its near side (this pixel nearer than its neighbours' mean: reverse Z, the second difference negative)
    if (abs(lx) > JUMP && lx > 0.0 && abs(ly) <= JUMP) || (abs(ly) > JUMP && ly > 0.0 && abs(lx) <= JUMP) || (lx > JUMP && ly > JUMP) {
        cover = 0.0;
    }
    let a = cover * clamp(outline.color.w, 0.0, 1.0);
    var c = max(outline.color.xyz, vec3f(0.0));
    if ENCODE_SRGB {
        c = srgb_encode(c);
    }
    return vec4f(c * a, a);
}

// ---------------------------------------------------------------- FXAA 3.11, quality preset 12

const SUBPIX: f32 = 0.75;
const EDGE_THRESHOLD: f32 = 0.166;
const EDGE_THRESHOLD_MIN: f32 = 0.0833;

fn luma(c: vec4f) -> f32 {
    let y = dot(clamp(c.xyz, vec3f(0.0), vec3f(1.0)), vec3f(0.299, 0.587, 0.114));
    if LINEAR {
        return sqrt(y);
    }
    return y;
}

fn tap(uv: vec2f) -> vec4f {
    return textureSampleLevel(image, smp, uv, 0.0);
}

fn luma_at(uv: vec2f) -> f32 {
    return luma(tap(uv));
}

// the step along an edge after i of them: 1, 1.5, 2, 4, 12 pixels (preset 12)
fn quality(i: u32) -> f32 {
    return select(select(select(select(12.0, 4.0, i == 3u), 2.0, i == 2u), 1.5, i == 1u), 1.0, i == 0u);
}

@fragment
fn fs_fxaa(@builtin(position) at: vec4f) -> @location(0) vec4f {
    let dim = vec2f(textureDimensions(image));
    let rcp = 1.0 / dim;
    let pos = at.xy * rcp;
    let p = vec2i(at.xy);
    let top = vec2i(dim) - 1;
    let center = textureLoad(image, p, 0);
    let lm = luma(center);
    let ls = luma(textureLoad(image, clamp(p + vec2i(0, 1), vec2i(0), top), 0));
    let le = luma(textureLoad(image, clamp(p + vec2i(1, 0), vec2i(0), top), 0));
    let ln = luma(textureLoad(image, clamp(p + vec2i(0, -1), vec2i(0), top), 0));
    let lw = luma(textureLoad(image, clamp(p + vec2i(-1, 0), vec2i(0), top), 0));
    let range_max = max(max(max(ln, lw), max(le, ls)), lm);
    let range_min = min(min(min(ln, lw), min(le, ls)), lm);
    let range = range_max - range_min;
    if range < max(EDGE_THRESHOLD_MIN, range_max * EDGE_THRESHOLD) {
        return center;
    }
    let lnw = luma(textureLoad(image, clamp(p + vec2i(-1, -1), vec2i(0), top), 0));
    let lse = luma(textureLoad(image, clamp(p + vec2i(1, 1), vec2i(0), top), 0));
    let lne = luma(textureLoad(image, clamp(p + vec2i(1, -1), vec2i(0), top), 0));
    let lsw = luma(textureLoad(image, clamp(p + vec2i(-1, 1), vec2i(0), top), 0));
    let lns = ln + ls;
    let lwe = lw + le;
    let edge_h1 = -2.0 * lm + lns;
    let edge_v1 = -2.0 * lm + lwe;
    let lnese = lne + lse;
    let lnwne = lnw + lne;
    let edge_h2 = -2.0 * le + lnese;
    let edge_v2 = -2.0 * ln + lnwne;
    let lnwsw = lnw + lsw;
    let lswse = lsw + lse;
    let edge_h4 = abs(edge_h1) * 2.0 + abs(edge_h2);
    let edge_v4 = abs(edge_v1) * 2.0 + abs(edge_v2);
    let edge_h3 = -2.0 * lw + lnwsw;
    let edge_v3 = -2.0 * ls + lswse;
    let edge_h = abs(edge_h3) + edge_h4;
    let edge_v = abs(edge_v3) + edge_v4;
    let horz = edge_h >= edge_v;
    let subpix_a = (lns + lwe) * 2.0 + lnwsw + lnese;
    // across the edge: north and south of a horizontal one, west and east of a vertical one
    var lo_n = ln;
    var lo_s = ls;
    var length_sign = rcp.y;
    if !horz {
        lo_n = lw;
        lo_s = le;
        length_sign = rcp.x;
    }
    let subpix_b = subpix_a * (1.0 / 12.0) - lm;
    let grad_n = lo_n - lm;
    let grad_s = lo_s - lm;
    var luma_nn = lo_n + lm;
    let luma_ss = lo_s + lm;
    let pair_n = abs(grad_n) >= abs(grad_s);
    let gradient = max(abs(grad_n), abs(grad_s));
    if pair_n {
        length_sign = -length_sign;
    }
    let subpix_c = clamp(abs(subpix_b) / range, 0.0, 1.0);
    var pos_b = pos;
    var off = vec2f(rcp.x, 0.0);
    if horz {
        pos_b.y = pos_b.y + length_sign * 0.5;
    } else {
        pos_b.x = pos_b.x + length_sign * 0.5;
        off = vec2f(0.0, rcp.y);
    }
    var pos_n = pos_b - off;
    var pos_p = pos_b + off;
    let subpix_d = -2.0 * subpix_c + 3.0;
    var end_n = luma_at(pos_n);
    let subpix_e = subpix_c * subpix_c;
    var end_p = luma_at(pos_p);
    if !pair_n {
        luma_nn = luma_ss;
    }
    let gradient_scaled = gradient * 0.25;
    let luma_mm = lm - luma_nn * 0.5;
    let subpix_f = subpix_d * subpix_e;
    let mm_below = luma_mm < 0.0;
    end_n = end_n - luma_nn * 0.5;
    end_p = end_p - luma_nn * 0.5;
    var done_n = abs(end_n) >= gradient_scaled;
    var done_p = abs(end_p) >= gradient_scaled;
    if !done_n {
        pos_n = pos_n - off * quality(1u);
    }
    if !done_p {
        pos_p = pos_p + off * quality(1u);
    }
    for (var i = 2u; i < 5u; i++) {
        if done_n && done_p {
            break;
        }
        if !done_n {
            end_n = luma_at(pos_n) - luma_nn * 0.5;
            done_n = abs(end_n) >= gradient_scaled;
            if !done_n {
                pos_n = pos_n - off * quality(i);
            }
        }
        if !done_p {
            end_p = luma_at(pos_p) - luma_nn * 0.5;
            done_p = abs(end_p) >= gradient_scaled;
            if !done_p {
                pos_p = pos_p + off * quality(i);
            }
        }
    }
    var dst_n = pos.x - pos_n.x;
    var dst_p = pos_p.x - pos.x;
    if !horz {
        dst_n = pos.y - pos_n.y;
        dst_p = pos_p.y - pos.y;
    }
    let good_n = (end_n < 0.0) != mm_below;
    let good_p = (end_p < 0.0) != mm_below;
    let span = dst_p + dst_n;
    let dir_n = dst_n < dst_p;
    let dst = min(dst_n, dst_p);
    let good = select(good_p, good_n, dir_n);
    let subpix_g = subpix_f * subpix_f;
    let offset = -dst / span + 0.5;
    let subpix_h = subpix_g * SUBPIX;
    let offset_good = select(0.0, offset, good);
    let shift = max(offset_good, subpix_h);
    var q = pos;
    if horz {
        q.y = q.y + shift * length_sign;
    } else {
        q.x = q.x + shift * length_sign;
    }
    return tap(q);
}
