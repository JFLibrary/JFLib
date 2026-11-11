// stroke: segments as quads with analytic coverage (decision 25; docs/RENDER2D.md "Der Shader").  No vertex buffer:
// a draw of the visible list of one layer, BATCH quads (four corners, u16 indices 0 1 2 2 1 3) per instance.
//
// The vertex shader places a segment relative to the camera (common.wgsl: rel2), in pixels, and clips it to the
// target grown by its reach: an end beyond that is open (no cap, no join, nothing of it shows).  Points far from the
// camera have f32's 24 bits of their distance only, so the line between two such points would wander; when both ends
// lie beyond FAR pixels, the line's place comes from the exact cross product of its ends in double-f32 (far_foot).
// The quad covers the segment's rectangle and how far its join's or cap's piece reaches beyond its ends, grown by a
// pixel for the smoothed edge.
//
// The fragment shader computes the share of the pixel (a unit square) inside the stroke, exactly: the area of the
// square inside convex pieces bounded by straight lines.  One edge cutting the pixel gives that half plane's share
// (cov, the box filter of a straight edge at any angle), the strip's two parallel edges alone cov(left) + cov(right)
// - 1 (a hairline too), more edges the area by Green's theorem (area6, up to six half planes).  A circle is cut by its
// tangent at the pixel's centre.  Most pixels lie in the body (the strip alone, no end near: two cov).
//
// Near a join the stroke is cut into tiles along the join's bisector: this segment's strip on its side of the
// bisector, the neighbour's on the other, a bevel's line cutting both; a round join's tile ends at the line at right
// angles through the vertex, the circle's sector beyond it is a piece of its own.  The tiles do not overlap and fill
// the stroke as long as the bisector's chord (h tan(phi / 2) along each segment) lies within both segments; then the
// shares add up exactly, and the outer side gets the miter's point, the bevel's corner or the round arc by itself.
// A sharper turn of a shorter segment takes the union instead: the rectangles, the join's piece beyond both their
// ends, |a| + |w| + |b| - |a and b| (a short segment near both its joins: both unions, the neighbours' rectangles
// meeting each other left out).  Caps: the rectangle grown by half a width (square), the half circle beyond its end
// (round).  The pieces near the pixel are bits of a mask, and one loop adds their shares, so that area6 sits in the
// shader once: inlined at forty places it made the shader so long that the GPU spent its time loading code.
//
// Overlap (docs/RENDER2D.md, "Überlappung"): every pixel near a join lies in both segments' quads, and a translucent
// polyline must not blend twice there.  No depth and no stencil: a pixel near a join belongs to the nearer of the two
// segments (their centre lines; near the vertex that is the bisector, beyond a short neighbour's end the longer
// one), and the other discards it.  Both compute the two distances from the same bits (the vertex from the same
// hi/lo, the segments as vectors the CPU rounded once: Record.info), so exactly one of them draws, with the union of
// both pieces.  Where a polyline comes back over itself away from a join its pieces blend each, as two entities
// would.
//
// Short thin strokes (TINY_LEN, TINY_H: mostly what a drawing shows of its details from far away) take a fast way near their
// ends: one segment's tile as the product of the strip's share and the share on its side of the bisector (the
// neighbour's piece likewise), instead of the exact area (docs/RENDER2D.md "Der schnelle Weg"); longer or wider strokes
// stay exact.
//
// Colours are sRGB codes blended as they are (premultiplied), as lib/gfx blends them.

const BATCH: u32 = 16384u;
const OPEN: u32 = 0u;
const CAP: u32 = 1u;
const JOIN: u32 = 2u;
// beyond this many pixels from the target's middle at both ends, a segment's line is placed by far_foot
const FAR: f32 = 131072.0;
// segments up to this long (px) and strokes up to this half width (px) take the fast way near their ends
const TINY_LEN: f32 = 3.0;
const TINY_H: f32 = 0.75;

struct Push {
    // the visible layer this draw shows (Basis; 0 on Voll, where first_instance carries it in its high 16 bits)
    layer: u32,
}

@group(0) @binding(0) var<uniform> view: View;
@group(0) @binding(1) var<storage, read> pool: array<Record>;
@group(0) @binding(2) var<storage, read> styles: array<Style>;
@group(0) @binding(3) var<storage, read> visible: array<u32>;
@group(0) @binding(4) var<storage, read> lists: array<vec4u>;
var<immediate> pc: Push;

struct VsOut {
    @builtin(position) pos: vec4f,
    // the segment's ends in pixels of the target (clipped where they lay beyond it)
    @location(0) @interpolate(flat) seg: vec4f,
    // in pixels: the previous point minus the start, the next point minus the end
    @location(1) @interpolate(flat) nb: vec4f,
    // half width (px), miter limit, the end minus the start in pixels (from Record.info, as the neighbours have it)
    @location(2) @interpolate(flat) par: vec4f,
    // start kind | end kind << 2 | cap << 4 | join << 8 | the polyline's ends << 12 (Record.info.w), colour
    @location(3) @interpolate(flat) misc: vec2u,
}

fn unit_or_zero(v: vec2f) -> vec2f {
    let l = length(v);
    if l > 0.0 {
        return v / l;
    }
    return vec2f(0.0);
}

// how far the pixels a segment draws reach beyond an end along it (px; the smoothed edge's pixel comes on top): a cap
// its half width (none for a butt cap); a join only at the segment's end, where it comes in: the join's piece
// beyond the end belongs to it (both segments' nearest point there is the vertex, and the incoming one wins), as far
// as that piece's farthest corner: the miter point h tan(phi / 2) or the outgoing outer corner h sin(phi), a bevel's
// corner h sin(phi), a circle h sin(phi) up to a right angle, then h.  dir points out of the segment, nbdir along the
// neighbour away from the end (a straight continuation: the same direction), c = cos(phi) of the turn
fn end_reach(dir: vec2f, nbdir: vec2f, joined: bool, incoming: bool, h: f32, st: Style) -> f32 {
    if !joined {
        return select(h, 0.0, cap_of(st) == CAP_BUTT);
    }
    if !incoming {
        return 0.0;
    }
    let c = clamp(dot(dir, nbdir), -1.0, 1.0);
    let sn = sqrt(max(1.0 - c * c, 0.0));
    let j = join_of(st);
    if j == JOIN_MITER && miter_ok(c, st.miter) {
        return h * max(sn, sqrt(max(1.0 - c, 0.0) / max(1.0 + c, 1e-12)));
    }
    if j == JOIN_ROUND && c < 0.0 {
        return h;
    }
    return h * sn;
}

// a * b exactly as p + e (the fused multiply-add keeps what the product rounded away)
fn two_prod(a: f32, b: f32) -> vec2f {
    let p = a * b;
    return vec2f(p, fma(a, b, -p));
}

// the foot of the camera's centre on the line through two points far from it (r0, r1: rel2's pairs): their cross
// product in double-f32 over their distance is the line's signed distance from the centre (px from the middle)
fn far_foot(r0: vec4f, r1: vec4f) -> vec2f {
    let a = two_prod(r0.x, r1.y);
    let b = two_prod(r0.y, r1.x);
    let s = two_sum(a.x, -b.x);
    let rest = (a.y - b.y) + ((r0.x * r1.w + r0.z * r1.y) - (r0.y * r1.z + r0.w * r1.x));
    let cr = s.x + (s.y + rest);
    let dv = vec2f(r1.x - r0.x, r1.y - r0.y);
    let lu = length(dv);
    let du = dv / lu;
    return to_px(-(cr / lu) * vec2f(-du.y, du.x), view);
}

@vertex
fn vs(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> VsOut {
    var o: VsOut;
    // a quad of nothing: all four corners in one point
    o.pos = vec4f(2.0, 2.0, 0.5, 1.0);
    o.seg = vec4f(0.0);
    o.nb = vec4f(0.0);
    o.par = vec4f(0.0);
    o.misc = vec2u(0u);
    let l = lists[pc.layer + (ii >> 16u)];
    let j = (ii & 0xffffu) * BATCH + (vi >> 2u);
    if j >= l.y {
        return o;
    }
    let r = pool[visible[l.x + j]];
    let st = styles[r.info.x - 1u];
    let h = half_width(st, view);
    let half = 0.5 * view.turn.zw;
    let r0 = rel2(r.a, view);
    let r1 = rel2(r.b, view);
    // pixels from the target's middle
    let p0 = to_px(r0.xy, view);
    let p1 = to_px(r1.xy, view);
    let cap = cap_of(st);
    var sk = select(CAP, JOIN, any(r.near.xy != vec2f(0.0)));
    var ek = select(CAP, JOIN, any(r.near.zw != vec2f(0.0)));
    let va = to_px(r.near.xy, view);
    let vb = to_px(r.near.zw, view);
    let own = to_px(bitcast<vec2f>(r.info.yz), view);
    let len0 = length(p1 - p0);
    var d = unit_or_zero(own);
    if !any(d != vec2f(0.0)) {
        d = vec2f(1.0, 0.0);
    }
    // a point without a cap: nothing
    if len0 == 0.0 && cap == CAP_BUTT && sk == CAP && ek == CAP {
        return o;
    }
    let e0 = end_reach(-d, unit_or_zero(va), sk == JOIN, false, h, st);
    let e1 = end_reach(d, unit_or_zero(vb), ek == JOIN, true, h, st);
    let m = max(h, max(e0, e1)) + 2.0;
    let lo = -half - vec2f(m);
    let hi = half + vec2f(m);
    // the line as base + s d, s from s0 to s1: the base its nearer end, or (both ends far) the foot of the centre
    var base = p0;
    var s0 = 0.0;
    var s1 = len0;
    if max(abs(p1.x), abs(p1.y)) < max(abs(p0.x), abs(p0.y)) {
        base = p1;
        s0 = -len0;
        s1 = 0.0;
    }
    let far = min(max(abs(p0.x), abs(p0.y)), max(abs(p1.x), abs(p1.y))) > FAR && max(hi.x, hi.y) < 0.25 * FAR;
    if far {
        base = far_foot(r0, r1);
        s0 = dot(p0 - base, d);
        s1 = dot(p1 - base, d);
    }
    // clipped to the target and its margin (Liang-Barsky on base + s d)
    var sa = s0;
    var sb = s1;
    for (var k = 0u; k < 2u; k++) {
        let b = base[k];
        let dk = d[k];
        if abs(dk) < 1e-12 {
            if b < lo[k] || b > hi[k] {
                return o;
            }
        } else {
            let ta = (lo[k] - b) / dk;
            let tb = (hi[k] - b) / dk;
            sa = max(sa, min(ta, tb));
            sb = min(sb, max(ta, tb));
        }
    }
    if sa > sb {
        return o;
    }
    var q0 = p0;
    var q1 = p1;
    if sa > s0 || far {
        q0 = base + sa * d;
        sk = OPEN;
    }
    if sb < s1 || far {
        q1 = base + sb * d;
        ek = OPEN;
    }
    let len = length(q1 - q0);
    let ext0 = select(e0 + 1.0, 0.0, sk == OPEN);
    let ext1 = select(e1 + 1.0, 0.0, ek == OPEN);
    let w = h + 1.0;
    let n = vec2f(-d.y, d.x);
    let u = select(-ext0, len + ext1, (vi & 1u) != 0u);
    let v = select(-w, w, (vi & 2u) != 0u);
    let p = q0 + u * d + v * n;
    o.pos = vec4f(p.x / half.x, -p.y / half.y, 0.5, 1.0);
    o.seg = vec4f(q0 + half, q1 + half);
    o.nb = vec4f(va, vb);
    o.par = vec4f(h, st.miter, own);
    o.misc = vec2u(sk | (ek << 2u) | (cap << 4u) | (join_of(st) << 8u) | ((r.info.w & 3u) << 12u), st.color);
    return o;
}

// the share of the pixel (a unit square around its centre) inside a half plane whose edge lies s from the centre (s >
// 0: the centre is outside), m the edge's outward unit normal: t = dot(x - centre, m) over the square is the sum of
// two uniform parts, |m.x| and |m.y| wide; this is its distribution at -s, exact for a straight edge
fn cov(s: f32, m: vec2f) -> f32 {
    let a = min(abs(m.x), abs(m.y));
    let b = max(abs(m.x), abs(m.y));
    let x = -s;
    let outer = 0.5 * (a + b);
    if x >= outer {
        return 1.0;
    }
    if x <= -outer {
        return 0.0;
    }
    if a < 1e-4 {
        return clamp(0.5 + x / b, 0.0, 1.0);
    }
    let inner = 0.5 * (b - a);
    if x < -inner {
        let t = x + outer;
        return t * t / (2.0 * a * b);
    }
    if x > inner {
        let t = outer - x;
        return 1.0 - t * t / (2.0 * a * b);
    }
    return 0.5 + x / b;
}

// (written by tools/gen_area6.py from the description below; unrolled on purpose: loops over planes and arrays of them cost a
// GPU a hundred times more here)
//
// the area of the pixel (the unit square around its centre) inside up to six half planes (m, s; unused ones ALL):
// none cutting it 1, one exactly by cov, the strip's two alone by cov + cov - 1, more by Green's theorem: the
// boundary is pieces of the square's sides and of the cutting planes' lines, each a segment narrowed to all the
// others, and the area half the sum of cross(start, end) over them, counter-clockwise; two lines on one line: one
// bounds the region when they face the same way, none when they face each other
fn area6(p0: vec3f, p1: vec3f, p2: vec3f, p3: vec3f, p4: vec3f, p5: vec3f) -> f32 {
    let k0 = kind_of(p0);
    let k1 = kind_of(p1);
    let k2 = kind_of(p2);
    let k3 = kind_of(p3);
    let k4 = kind_of(p4);
    let k5 = kind_of(p5);
    if k0 == 2u || k1 == 2u || k2 == 2u || k3 == 2u || k4 == 2u || k5 == 2u {
        return 0.0;
    }
    var n = 0u;
    var c0 = ALL;
    var c1 = ALL;
    var c2 = ALL;
    var c3 = ALL;
    var c4 = ALL;
    var c5 = ALL;
    if k0 == 1u {
        c0 = select(c0, p0, n == 0u);
        c1 = select(c1, p0, n == 1u);
        c2 = select(c2, p0, n == 2u);
        c3 = select(c3, p0, n == 3u);
        c4 = select(c4, p0, n == 4u);
        c5 = select(c5, p0, n == 5u);
        n += 1u;
    }
    if k1 == 1u {
        c0 = select(c0, p1, n == 0u);
        c1 = select(c1, p1, n == 1u);
        c2 = select(c2, p1, n == 2u);
        c3 = select(c3, p1, n == 3u);
        c4 = select(c4, p1, n == 4u);
        c5 = select(c5, p1, n == 5u);
        n += 1u;
    }
    if k2 == 1u {
        c0 = select(c0, p2, n == 0u);
        c1 = select(c1, p2, n == 1u);
        c2 = select(c2, p2, n == 2u);
        c3 = select(c3, p2, n == 3u);
        c4 = select(c4, p2, n == 4u);
        c5 = select(c5, p2, n == 5u);
        n += 1u;
    }
    if k3 == 1u {
        c0 = select(c0, p3, n == 0u);
        c1 = select(c1, p3, n == 1u);
        c2 = select(c2, p3, n == 2u);
        c3 = select(c3, p3, n == 3u);
        c4 = select(c4, p3, n == 4u);
        c5 = select(c5, p3, n == 5u);
        n += 1u;
    }
    if k4 == 1u {
        c0 = select(c0, p4, n == 0u);
        c1 = select(c1, p4, n == 1u);
        c2 = select(c2, p4, n == 2u);
        c3 = select(c3, p4, n == 3u);
        c4 = select(c4, p4, n == 4u);
        c5 = select(c5, p4, n == 5u);
        n += 1u;
    }
    if k5 == 1u {
        c0 = select(c0, p5, n == 0u);
        c1 = select(c1, p5, n == 1u);
        c2 = select(c2, p5, n == 2u);
        c3 = select(c3, p5, n == 3u);
        c4 = select(c4, p5, n == 4u);
        c5 = select(c5, p5, n == 5u);
        n += 1u;
    }
    if n == 0u {
        return 1.0;
    }
    if n == 1u {
        return cov(c0.z, c0.xy);
    }
    if n == 2u && dot(c0.xy, c1.xy) < -0.99999 {
        return max(cov(c0.z, c0.xy) + cov(c1.z, c1.xy) - 1.0, 0.0);
    }
    var a = 0.0;
    // the square's sides
    {
        let sp = vec2f(-0.5, -0.5);
        let sd = vec2f(1.0, 0.0);
        var t = vec2f(0.0, 1.0);
        if n > 0u {
            t = narrow(sp, sd, c0, t);
        }
        if n > 1u {
            t = narrow(sp, sd, c1, t);
        }
        if n > 2u {
            t = narrow(sp, sd, c2, t);
        }
        if n > 3u {
            t = narrow(sp, sd, c3, t);
        }
        if n > 4u {
            t = narrow(sp, sd, c4, t);
        }
        if n > 5u {
            t = narrow(sp, sd, c5, t);
        }
        a += piece(sp, sd, t);
    }
    {
        let sp = vec2f(0.5, -0.5);
        let sd = vec2f(0.0, 1.0);
        var t = vec2f(0.0, 1.0);
        if n > 0u {
            t = narrow(sp, sd, c0, t);
        }
        if n > 1u {
            t = narrow(sp, sd, c1, t);
        }
        if n > 2u {
            t = narrow(sp, sd, c2, t);
        }
        if n > 3u {
            t = narrow(sp, sd, c3, t);
        }
        if n > 4u {
            t = narrow(sp, sd, c4, t);
        }
        if n > 5u {
            t = narrow(sp, sd, c5, t);
        }
        a += piece(sp, sd, t);
    }
    {
        let sp = vec2f(0.5, 0.5);
        let sd = vec2f(-1.0, 0.0);
        var t = vec2f(0.0, 1.0);
        if n > 0u {
            t = narrow(sp, sd, c0, t);
        }
        if n > 1u {
            t = narrow(sp, sd, c1, t);
        }
        if n > 2u {
            t = narrow(sp, sd, c2, t);
        }
        if n > 3u {
            t = narrow(sp, sd, c3, t);
        }
        if n > 4u {
            t = narrow(sp, sd, c4, t);
        }
        if n > 5u {
            t = narrow(sp, sd, c5, t);
        }
        a += piece(sp, sd, t);
    }
    {
        let sp = vec2f(-0.5, 0.5);
        let sd = vec2f(0.0, -1.0);
        var t = vec2f(0.0, 1.0);
        if n > 0u {
            t = narrow(sp, sd, c0, t);
        }
        if n > 1u {
            t = narrow(sp, sd, c1, t);
        }
        if n > 2u {
            t = narrow(sp, sd, c2, t);
        }
        if n > 3u {
            t = narrow(sp, sd, c3, t);
        }
        if n > 4u {
            t = narrow(sp, sd, c4, t);
        }
        if n > 5u {
            t = narrow(sp, sd, c5, t);
        }
        a += piece(sp, sd, t);
    }
    // the cutting planes' lines
    if n > 0u {
        let lp = -c0.z * c0.xy;
        let ld = vec2f(-c0.y, c0.x);
        var t = vec2f(-1.0, 1.0);
        t = narrow(lp, ld, vec3f(1.0, 0.0, -0.5), t);
        t = narrow(lp, ld, vec3f(-1.0, 0.0, -0.5), t);
        t = narrow(lp, ld, vec3f(0.0, 1.0, -0.5), t);
        t = narrow(lp, ld, vec3f(0.0, -1.0, -0.5), t);
        if n > 1u {
            t = narrow_line(lp, ld, c0.xy, c1, false, t);
        }
        if n > 2u {
            t = narrow_line(lp, ld, c0.xy, c2, false, t);
        }
        if n > 3u {
            t = narrow_line(lp, ld, c0.xy, c3, false, t);
        }
        if n > 4u {
            t = narrow_line(lp, ld, c0.xy, c4, false, t);
        }
        if n > 5u {
            t = narrow_line(lp, ld, c0.xy, c5, false, t);
        }
        a += piece(lp, ld, t);
    }
    if n > 1u {
        let lp = -c1.z * c1.xy;
        let ld = vec2f(-c1.y, c1.x);
        var t = vec2f(-1.0, 1.0);
        t = narrow(lp, ld, vec3f(1.0, 0.0, -0.5), t);
        t = narrow(lp, ld, vec3f(-1.0, 0.0, -0.5), t);
        t = narrow(lp, ld, vec3f(0.0, 1.0, -0.5), t);
        t = narrow(lp, ld, vec3f(0.0, -1.0, -0.5), t);
        if n > 0u {
            t = narrow_line(lp, ld, c1.xy, c0, true, t);
        }
        if n > 2u {
            t = narrow_line(lp, ld, c1.xy, c2, false, t);
        }
        if n > 3u {
            t = narrow_line(lp, ld, c1.xy, c3, false, t);
        }
        if n > 4u {
            t = narrow_line(lp, ld, c1.xy, c4, false, t);
        }
        if n > 5u {
            t = narrow_line(lp, ld, c1.xy, c5, false, t);
        }
        a += piece(lp, ld, t);
    }
    if n > 2u {
        let lp = -c2.z * c2.xy;
        let ld = vec2f(-c2.y, c2.x);
        var t = vec2f(-1.0, 1.0);
        t = narrow(lp, ld, vec3f(1.0, 0.0, -0.5), t);
        t = narrow(lp, ld, vec3f(-1.0, 0.0, -0.5), t);
        t = narrow(lp, ld, vec3f(0.0, 1.0, -0.5), t);
        t = narrow(lp, ld, vec3f(0.0, -1.0, -0.5), t);
        if n > 0u {
            t = narrow_line(lp, ld, c2.xy, c0, true, t);
        }
        if n > 1u {
            t = narrow_line(lp, ld, c2.xy, c1, true, t);
        }
        if n > 3u {
            t = narrow_line(lp, ld, c2.xy, c3, false, t);
        }
        if n > 4u {
            t = narrow_line(lp, ld, c2.xy, c4, false, t);
        }
        if n > 5u {
            t = narrow_line(lp, ld, c2.xy, c5, false, t);
        }
        a += piece(lp, ld, t);
    }
    if n > 3u {
        let lp = -c3.z * c3.xy;
        let ld = vec2f(-c3.y, c3.x);
        var t = vec2f(-1.0, 1.0);
        t = narrow(lp, ld, vec3f(1.0, 0.0, -0.5), t);
        t = narrow(lp, ld, vec3f(-1.0, 0.0, -0.5), t);
        t = narrow(lp, ld, vec3f(0.0, 1.0, -0.5), t);
        t = narrow(lp, ld, vec3f(0.0, -1.0, -0.5), t);
        if n > 0u {
            t = narrow_line(lp, ld, c3.xy, c0, true, t);
        }
        if n > 1u {
            t = narrow_line(lp, ld, c3.xy, c1, true, t);
        }
        if n > 2u {
            t = narrow_line(lp, ld, c3.xy, c2, true, t);
        }
        if n > 4u {
            t = narrow_line(lp, ld, c3.xy, c4, false, t);
        }
        if n > 5u {
            t = narrow_line(lp, ld, c3.xy, c5, false, t);
        }
        a += piece(lp, ld, t);
    }
    if n > 4u {
        let lp = -c4.z * c4.xy;
        let ld = vec2f(-c4.y, c4.x);
        var t = vec2f(-1.0, 1.0);
        t = narrow(lp, ld, vec3f(1.0, 0.0, -0.5), t);
        t = narrow(lp, ld, vec3f(-1.0, 0.0, -0.5), t);
        t = narrow(lp, ld, vec3f(0.0, 1.0, -0.5), t);
        t = narrow(lp, ld, vec3f(0.0, -1.0, -0.5), t);
        if n > 0u {
            t = narrow_line(lp, ld, c4.xy, c0, true, t);
        }
        if n > 1u {
            t = narrow_line(lp, ld, c4.xy, c1, true, t);
        }
        if n > 2u {
            t = narrow_line(lp, ld, c4.xy, c2, true, t);
        }
        if n > 3u {
            t = narrow_line(lp, ld, c4.xy, c3, true, t);
        }
        if n > 5u {
            t = narrow_line(lp, ld, c4.xy, c5, false, t);
        }
        a += piece(lp, ld, t);
    }
    if n > 5u {
        let lp = -c5.z * c5.xy;
        let ld = vec2f(-c5.y, c5.x);
        var t = vec2f(-1.0, 1.0);
        t = narrow(lp, ld, vec3f(1.0, 0.0, -0.5), t);
        t = narrow(lp, ld, vec3f(-1.0, 0.0, -0.5), t);
        t = narrow(lp, ld, vec3f(0.0, 1.0, -0.5), t);
        t = narrow(lp, ld, vec3f(0.0, -1.0, -0.5), t);
        if n > 0u {
            t = narrow_line(lp, ld, c5.xy, c0, true, t);
        }
        if n > 1u {
            t = narrow_line(lp, ld, c5.xy, c1, true, t);
        }
        if n > 2u {
            t = narrow_line(lp, ld, c5.xy, c2, true, t);
        }
        if n > 3u {
            t = narrow_line(lp, ld, c5.xy, c3, true, t);
        }
        if n > 4u {
            t = narrow_line(lp, ld, c5.xy, c4, true, t);
        }
        a += piece(lp, ld, t);
    }
    return clamp(0.5 * a, 0.0, 1.0);
}

// a half plane with everything inside: an unused place
const ALL: vec3f = vec3f(1.0, 0.0, -1e30);
// how close to a pixel's corner a half plane's edge must come to count as cutting it (a line along a pixel's side by a
// rounding error does not)
const EDGE_EPS: f32 = 1e-5;

// what a half plane does to the pixel: 0 holds it all, 1 cuts it, 2 holds nothing of it
fn kind_of(p: vec3f) -> u32 {
    let r = 0.5 * (abs(p.x) + abs(p.y)) - EDGE_EPS;
    if p.z >= r {
        return 2u;
    }
    if p.z > -r {
        return 1u;
    }
    return 0u;
}

// a segment p + t d (t from t.x to t.y) narrowed to the half plane dot(x, m) + s <= 0 (pl); empty: t.x >= t.y
fn narrow(p: vec2f, d: vec2f, pl: vec3f, t: vec2f) -> vec2f {
    let num = dot(p, pl.xy) + pl.z;
    let den = dot(d, pl.xy);
    if den == 0.0 {
        if num > 0.0 {
            return vec2f(1.0, 0.0);
        }
        return t;
    }
    let tc = -num / den;
    if den > 0.0 {
        return vec2f(t.x, min(t.y, tc));
    }
    return vec2f(max(t.x, tc), t.y);
}

// the same for a plane's own line (normal m) against another plane o: on one line, it bounds the region only when it
// faces o's way and comes first (earlier false), never when they face each other
fn narrow_line(p: vec2f, d: vec2f, m: vec2f, o: vec3f, earlier: bool, t: vec2f) -> vec2f {
    if abs(dot(d, o.xy)) < 1e-6 && abs(dot(p, o.xy) + o.z) < 1e-5 {
        if dot(m, o.xy) < 0.0 || earlier {
            return vec2f(1.0, 0.0);
        }
        return t;
    }
    return narrow(p, d, o, t);
}

// cross(start, end) of a boundary piece p + t d, t from t.x to t.y (0 when empty)
fn piece(p: vec2f, d: vec2f, t: vec2f) -> f32 {
    if t.y <= t.x {
        return 0.0;
    }
    let a = p + t.x * d;
    let b = p + t.y * d;
    return a.x * b.y - b.x * a.y;
}

// the tangent of the circle of radius hw around c at pixel q, as a half plane
fn tangent(q: vec2f, c: vec2f, hw: f32) -> vec3f {
    let r = q - c;
    let l = length(r);
    var m = vec2f(1.0, 0.0);
    if l > 1e-6 {
        m = r / l;
    }
    return vec3f(m, l - hw);
}

// the line at right angles through point c with the inside towards -dir (the half plane dot(x - c, dir) <= 0)
fn before(q: vec2f, c: vec2f, dir: vec2f) -> vec3f {
    return vec3f(dir, dot(q - c, dir));
}

// a cap at end point c, dir pointing out of the segment: the line that ends its rectangle (a square cap half a width
// further), and (a round cap) the half circle beyond it as its own piece
fn cap_line(q: vec2f, c: vec2f, dir: vec2f, cap: u32, h: f32) -> vec3f {
    let p = before(q, c, dir);
    return select(p, p - vec3f(0.0, 0.0, h), cap == CAP_SQUARE);
}

// a join at vertex v between the segment coming in (din) and the one going out (dout), unit directions
struct Join {
    v: vec2f,
    din: vec2f,
    dout: vec2f,
    // the bisector's normal: the incoming tile is where dot(q - v, m) <= 0
    m: vec2f,
    // the bisector of the outer normals, along which a bevel's line lies off from v
    w: vec2f,
    off: f32,
    // JOIN_MITER (within its limit), JOIN_ROUND, JOIN_BEVEL (also a miter beyond its limit)
    kind: u32,
    // the bisector's chord reaches h tan(phi / 2) along each segment from v
    reach: f32,
}

fn make_join(v: vec2f, din: vec2f, dout: vec2f, kind: u32, limit: f32, h: f32) -> Join {
    var j: Join;
    j.v = v;
    j.din = din;
    j.dout = dout;
    let c = clamp(dot(din, dout), -1.0, 1.0);
    // (a turn to the left of din, or back: the outer side is the right one)
    let left = din.x * dout.y - din.y * dout.x >= 0.0;
    let s = din + dout;
    let ls = length(s);
    // (a turn back: the centre line splits the two strips, the incoming tile on the outer side)
    j.m = select(vec2f(-din.y, din.x), s / ls, ls > 1e-6);
    j.kind = kind;
    if kind == JOIN_MITER && !miter_ok(c, limit) {
        j.kind = JOIN_BEVEL;
    }
    let sg = select(1.0, -1.0, left);
    let so = sg * (vec2f(-din.y, din.x) + vec2f(-dout.y, dout.x));
    let lo = length(so);
    j.w = select(din, so / lo, lo > 1e-6);
    j.off = 0.5 * h * lo;
    j.reach = h * sqrt(max(1.0 - c, 0.0) / max(1.0 + c, 1e-12));
    return j;
}

// a join's bisector as a half plane: the incoming tile's side, or the outgoing one's
fn bisector(q: vec2f, j: Join, incoming: bool) -> vec3f {
    let b = vec3f(j.m, dot(q - j.v, j.m));
    return select(-b, b, incoming);
}

// a join's outer cut of both tiles: a bevel's line (else ALL)
fn bevel(q: vec2f, j: Join) -> vec3f {
    return select(ALL, vec3f(j.w, dot(q - j.v, j.w) - j.off), j.kind == JOIN_BEVEL);
}

// a join's piece beyond both rectangles' ends: a miter's quadrilateral, a bevel's triangle, a round join's sector
struct Planes4 {
    a: vec3f,
    b: vec3f,
    c: vec3f,
    d: vec3f,
}

fn wedge(q: vec2f, j: Join, h: f32) -> Planes4 {
    let r = q - j.v;
    var w3 = vec3f(j.w, dot(r, j.w) - j.off);
    var w4 = ALL;
    if j.kind == JOIN_ROUND {
        w3 = tangent(q, j.v, h);
    } else if j.kind == JOIN_MITER {
        let sg = select(1.0, -1.0, j.din.x * j.dout.y - j.din.y * j.dout.x >= 0.0);
        let oi = sg * vec2f(-j.din.y, j.din.x);
        let oo = sg * vec2f(-j.dout.y, j.dout.x);
        w3 = vec3f(oi, dot(r, oi) - h);
        w4 = vec3f(oo, dot(r, oo) - h);
    }
    return Planes4(vec3f(-j.din, -dot(r, j.din)), vec3f(j.dout, dot(r, j.dout)), w3, w4);
}

// the squared distance of a point r (relative to an end of a segment) from the segment from there along w; both
// segments at a join compute it from the same bits (r from the vertex, w from vectors the CPU rounded once)
fn dist_seg(r: vec2f, w: vec2f) -> f32 {
    let ww = dot(w, w);
    var t = 0.0;
    if ww > 0.0 {
        t = clamp(dot(r, w) / ww, 0.0, 1.0);
    }
    let x = r - t * w;
    return dot(x, x);
}

// the strip of a segment through point o along d: its two lines
fn strip_l(q: vec2f, o: vec2f, d: vec2f, h: f32) -> vec3f {
    let n = vec2f(-d.y, d.x);
    return vec3f(n, dot(q - o, n) - h);
}

fn strip_r(q: vec2f, o: vec2f, d: vec2f, h: f32) -> vec3f {
    let n = vec2f(-d.y, d.x);
    return vec3f(-n, -dot(q - o, n) - h);
}

// the pieces of the stroke near a pixel (the fragment shader's mask): this segment's rectangle, the previous and the
// next one's, round joins' circles beyond the lines at right angles (this side and the neighbour's of each vertex), the
// joins' pieces of the union, the rectangles meeting (taken away), round caps' half circles (own ends, the neighbours'
// far ends)
const S_OWN: u32 = 1u;
const S_PREV: u32 = 2u;
const S_NEXT: u32 = 4u;
const S_SECT_S_OWN: u32 = 8u;
const S_SECT_E_OWN: u32 = 16u;
const S_SECT_S_PREV: u32 = 32u;
const S_SECT_E_NEXT: u32 = 64u;
const S_WEDGE_S: u32 = 128u;
const S_WEDGE_E: u32 = 256u;
const S_X_PREV: u32 = 512u;
const S_X_NEXT: u32 = 1024u;
const S_HD_S: u32 = 2048u;
const S_HD_E: u32 = 4096u;
const S_HD_A: u32 = 8192u;
const S_HD_B: u32 = 16384u;

@fragment
fn fs(i: VsOut) -> @location(0) vec4f {
    let q = i.pos.xy;
    let s = i.seg.xy;
    let e = i.seg.zw;
    let h = i.par.x;
    let sk = i.misc.x & 3u;
    let ek = (i.misc.x >> 2u) & 3u;
    let cap = (i.misc.x >> 4u) & 15u;
    let jk = (i.misc.x >> 8u) & 15u;
    let ends = (i.misc.x >> 12u) & 3u;
    let own = i.par.zw;
    // a pixel near a join is the nearer segment's (the incoming one's when equal): see the head
    var dprev = 3.4e38;
    var dnext = 3.4e38;
    if sk == JOIN {
        dprev = dist_seg(q - s, i.nb.xy);
        if !(dist_seg(q - s, own) < dprev) {
            discard;
            return vec4f(0.0);
        }
    }
    if ek == JOIN {
        dnext = dist_seg(q - e, i.nb.zw);
        if !(dist_seg(q - e, -own) <= dnext) {
            discard;
            return vec4f(0.0);
        }
    }
    var d = unit_or_zero(own);
    if !any(d != vec2f(0.0)) {
        d = vec2f(1.0, 0.0);
    }
    let col = unpack4x8unorm(i.misc.y);
    let dp = unit_or_zero(-i.nb.xy);
    let dn = unit_or_zero(i.nb.zw);
    // which pieces may touch the pixel: a neighbour's rectangle (a square cap too) lies within h sqrt(2) of its segment,
    // a join's piece within its farthest corner's distance from the vertex (a miter's point h / cos(phi / 2), else h),
    // an own cap within h of its end; a pixel reaches 0.71 beyond its centre
    let rr = h * 1.415 + 0.71;
    var near_s = false;
    var near_e = false;
    if sk == JOIN {
        let c = dot(dp, d);
        let wr = h * select(1.0, max(sqrt(2.0 / max(1.0 + c, 1e-6)), 1.0), jk == JOIN_MITER && miter_ok(c, i.par.y)) + 0.71;
        near_s = dprev < rr * rr || length(q - s) < wr;
    }
    if ek == JOIN {
        let c = dot(d, dn);
        let wr = h * select(1.0, max(sqrt(2.0 / max(1.0 + c, 1e-6)), 1.0), jk == JOIN_MITER && miter_ok(c, i.par.y)) + 0.71;
        near_e = dnext < rr * rr || length(q - e) < wr;
    }
    let u0 = dot(q - s, d);
    let u1 = dot(q - e, d);
    // (a pixel beyond a join's end is never the body's: the join decides there)
    near_s = near_s || (sk == JOIN && u0 < 0.71);
    near_e = near_e || (ek == JOIN && u1 > -0.71);
    let lim = 0.71 + select(0.0, h, cap != CAP_BUTT);
    let near_cs = sk == CAP && u0 < lim;
    let near_ce = ek == CAP && u1 > -lim;
    let sl = strip_l(q, s, d, h);
    let sr = strip_r(q, s, d, h);
    if !near_s && !near_e && !near_cs && !near_ce {
        // the body: the strip alone (most pixels)
        let c0 = max(cov(sl.z, sl.xy) + cov(sr.z, sr.xy) - 1.0, 0.0);
        if !(c0 > 0.0) {
            discard;
            return vec4f(0.0);
        }
        return vec4f(vec3f(col.z, col.y, col.x) * (col.w * c0), col.w * c0);
    }
    // short thin strokes near their ends: the tile of each piece as a product (the strip's share times the share on its side
    // of the bisector), ownership and the near tests as above
    if h <= TINY_H && length(own) <= TINY_LEN {
        var cn = 0.0;
        var weight = 1.0;
        if near_s {
            let sm = dp + d;
            let ls = length(sm);
            let m = select(vec2f(-dp.y, dp.x), sm / ls, ls > 1e-6);
            let b = cov(-dot(q - s, m), m);
            let np = vec2f(-dp.y, dp.x);
            let vp = dot(q - s, np);
            cn += (1.0 - b) * max(cov(vp - h, np) + cov(-vp - h, -np) - 1.0, 0.0);
            weight *= b;
        }
        if near_e {
            let sm = d + dn;
            let ls = length(sm);
            let m = select(vec2f(-d.y, d.x), sm / ls, ls > 1e-6);
            let b = cov(dot(q - e, m), -m);
            let nx = vec2f(-dn.y, dn.x);
            let vn = dot(q - e, nx);
            cn += (1.0 - b) * max(cov(vn - h, nx) + cov(-vn - h, -nx) - 1.0, 0.0);
            weight *= b;
        }
        var o = max(cov(sl.z, sl.xy) + cov(sr.z, sr.xy) - 1.0, 0.0);
        // caps: a butt line at the end, a square one half a width on, a round one where a line would keep its area
        let ext = select(select(0.0, h, cap == CAP_SQUARE), 0.7854 * h, cap == CAP_ROUND);
        if sk == CAP {
            o *= cov(-u0 - ext, -d);
        }
        if ek == CAP {
            o *= cov(u1 - ext, d);
        }
        let ct = clamp(cn + weight * o, 0.0, 1.0);
        if !(ct > 0.0) {
            discard;
            return vec4f(0.0);
        }
        return vec4f(vec3f(col.z, col.y, col.x) * (col.w * ct), col.w * ct);
    }
    // its own ends' lines: a cap's, a join's line at right angles (that join not near), none where open
    let ps = select(select(cap_line(q, s, -d, cap, h), before(q, s, -d), sk == JOIN), ALL, sk == OPEN);
    let pe = select(select(cap_line(q, e, d, cap, h), before(q, e, d), ek == JOIN), ALL, ek == OPEN);
    let lens = length(own);
    let a = s + i.nb.xy;
    let b = e + i.nb.zw;
    let cp = (ends & 1u) != 0u;
    let cn = (ends & 2u) != 0u;
    let round = cap == CAP_ROUND;
    var js: Join;
    var je: Join;
    var ok = true;
    if near_s {
        js = make_join(s, dp, d, jk, i.par.y, h);
        ok = ok && js.reach <= min(lens, length(i.nb.xy));
    }
    if near_e {
        je = make_join(e, d, dn, jk, i.par.y, h);
        ok = ok && je.reach <= min(lens, length(i.nb.zw));
    }
    // the pieces that may touch the pixel, as bits of a mask (S_* below), and the cuts that depend on the case: this
    // segment's rectangle [strip, o2, o3, o4, o5], the previous one's [strip, pf, pn, p4], the next one's [strip, nn,
    // nf, n4]; the loop below adds (or takes away) each piece's share, so that area6 sits in the shader once (written
    // out at every place it was 40 times, and the GPU spent its time loading code)
    var o2 = ps;
    var o3 = pe;
    var o4 = ALL;
    var o5 = ALL;
    var pf = ALL;
    var pn = ALL;
    var p4 = ALL;
    var nn = ALL;
    var nf = ALL;
    var n4 = ALL;
    var mask = S_OWN;
    let jround = jk == JOIN_ROUND;
    if round {
        mask |= select(0u, S_HD_S, near_cs) | select(0u, S_HD_E, near_ce);
    }
    if ok && (near_s || near_e) {
        // the tiles beside the near joins (their bisectors' chords within both segments: they do not overlap and fill
        // the stroke there): this segment's, the neighbours'; a round join's circle beyond the line at right angles
        // through the vertex, a bevel's line
        if near_s {
            o2 = bisector(q, js, false);
            o4 = select(bevel(q, js), before(q, s, -d), jround);
            pf = select(ALL, cap_line(q, a, -dp, cap, h), cp);
            pn = bisector(q, js, true);
            p4 = select(bevel(q, js), before(q, s, dp), jround);
            mask |= S_PREV | select(0u, S_SECT_S_OWN | S_SECT_S_PREV, jround);
        }
        if near_e {
            o3 = bisector(q, je, true);
            o5 = select(bevel(q, je), before(q, e, d), jround);
            nn = bisector(q, je, false);
            nf = select(ALL, cap_line(q, b, dn, cap, h), cn);
            n4 = select(bevel(q, je), before(q, e, -dn), jround);
            mask |= S_NEXT | select(0u, S_SECT_E_OWN | S_SECT_E_NEXT, jround);
        }
    } else if near_s || near_e {
        // a join whose bisector's chord is longer than a segment (a sharp turn of a short one): the union of the
        // rectangles and the joins' pieces (beyond both rectangles' ends: they meet none), |a| + |w| + |b| - |a and b|,
        // where a short segment near both joins leaves out the neighbours' rectangles meeting each other
        if near_s {
            o2 = before(q, s, -d);
            pf = select(select(ALL, before(q, a, -dp), near_e), cap_line(q, a, -dp, cap, h), cp);
            pn = before(q, s, dp);
            mask |= S_PREV | S_WEDGE_S | S_X_PREV;
        }
        if near_e {
            o3 = before(q, e, d);
            nn = before(q, e, -dn);
            nf = select(select(ALL, before(q, b, dn), near_s), cap_line(q, b, dn, cap, h), cn);
            mask |= S_NEXT | S_WEDGE_E | S_X_NEXT;
        }
    }
    // (a round cap at the polyline's end, on a near neighbour)
    if round {
        mask |= select(0u, S_HD_A, near_s && cp && length(q - a) < h + 0.71) | select(0u, S_HD_B, near_e && cn && length(q - b) < h + 0.71);
    }
    var c = 0.0;
    var ao = 0.0;
    var ap = 0.0;
    var an = 0.0;
    var m = mask;
    loop {
        if m == 0u {
            break;
        }
        let k = firstTrailingBit(m);
        m &= m - 1u;
        var x0 = ALL;
        var x1 = ALL;
        var x2 = ALL;
        var x3 = ALL;
        var x4 = ALL;
        var x5 = ALL;
        var w = 1.0;
        switch k {
            case 0u {
                x0 = sl;
                x1 = sr;
                x2 = o2;
                x3 = o3;
                x4 = o4;
                x5 = o5;
            }
            case 1u {
                x0 = strip_l(q, s, dp, h);
                x1 = strip_r(q, s, dp, h);
                x2 = pf;
                x3 = pn;
                x4 = p4;
            }
            case 2u {
                x0 = strip_l(q, e, dn, h);
                x1 = strip_r(q, e, dn, h);
                x2 = nn;
                x3 = nf;
                x4 = n4;
            }
            case 3u {
                x0 = -before(q, s, -d);
                x1 = bisector(q, js, false);
                x2 = tangent(q, s, h);
            }
            case 4u {
                x0 = -before(q, e, d);
                x1 = bisector(q, je, true);
                x2 = tangent(q, e, h);
            }
            case 5u {
                x0 = -before(q, s, dp);
                x1 = bisector(q, js, true);
                x2 = tangent(q, s, h);
            }
            case 6u {
                x0 = -before(q, e, -dn);
                x1 = bisector(q, je, false);
                x2 = tangent(q, e, h);
            }
            case 7u, 8u {
                var j = js;
                if k == 8u {
                    j = je;
                }
                let wd = wedge(q, j, h);
                x0 = wd.a;
                x1 = wd.b;
                x2 = wd.c;
                x3 = wd.d;
            }
            case 9u {
                // (the rectangles meeting: this one's and the previous one's beside the vertex)
                w = select(0.0, -1.0, ao > 0.0 && ap > 0.0);
                x0 = sl;
                x1 = sr;
                x2 = before(q, s, -d);
                x3 = strip_l(q, s, dp, h);
                x4 = strip_r(q, s, dp, h);
                x5 = before(q, s, dp);
            }
            case 10u {
                w = select(0.0, -1.0, ao > 0.0 && an > 0.0);
                x0 = sl;
                x1 = sr;
                x2 = before(q, e, d);
                x3 = strip_l(q, e, dn, h);
                x4 = strip_r(q, e, dn, h);
                x5 = before(q, e, -dn);
            }
            default {
                // a round cap's half circle: at either own end, at the previous or next segment's far end
                let ctr = select(select(b, a, k == 13u), select(e, s, k == 11u), k <= 12u);
                let out = select(select(dn, -dp, k == 13u), select(d, -d, k == 11u), k <= 12u);
                x0 = tangent(q, ctr, h);
                x1 = before(q, ctr, -out);
            }
        }
        if w != 0.0 {
            let x = area6(x0, x1, x2, x3, x4, x5);
            if k == 0u {
                ao = x;
            } else if k == 1u {
                ap = x;
            } else if k == 2u {
                an = x;
            }
            c += w * x;
        }
    }
    c = clamp(c, 0.0, 1.0);
    if !(c > 0.0) {
        discard;
        return vec4f(0.0);
    }
    let al = col.w * c;
    return vec4f(vec3f(col.z, col.y, col.x) * al, al);
}
