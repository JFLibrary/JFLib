// stroke: segments and arcs as quads with analytic coverage (decision 25; docs/RENDER2D.md "Der Shader").  No vertex
// buffer: a draw of the visible list of one layer, BATCH quads (four corners, u16 indices 0 1 2 2 1 3) per instance.
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
// solid path once: inlined at forty places it made the shader so long that the GPU spent its time loading code.
//
// Overlap (docs/RENDER2D.md, "Überlappung"): every pixel near a join lies in both segments' quads, and a translucent
// polyline must not blend twice there.  No depth and no stencil: a pixel near a join belongs to the nearer of the two
// segments (their centre lines; near the vertex that is the bisector, beyond a short neighbour's end the longer
// one), and the other discards it.  Both compute the two distances from the same bits (the vertex from the same
// hi/lo, the segments as vectors the CPU rounded once: Record.v), so exactly one of them draws, with the union of
// both pieces.  Where a polyline comes back over itself away from a join its pieces blend each, as two entities
// would.
//
// Short thin strokes (TINY_LEN, TINY_H: mostly what a drawing shows of its details from far away) take a fast way near
// their ends: one segment's tile as the product of the strip's share and the share on its side of the bisector (the
// neighbour's piece likewise), instead of the exact area (docs/RENDER2D.md "Der schnelle Weg"); longer or wider strokes
// stay exact.
//
// Arcs (vs_arc, fs_arc; docs/RENDER2D.md "Bögen und Kreise"): an arc is a head and pieces of at most 22.5 degrees, each
// a record with its circle's centre and its end, its start the end of the record before.  The circle near the camera
// comes from |v|^2 - r^2 in double-f32 (v the camera minus the centre): F, the point of the circle nearest the camera's
// centre, its outward unit u, the radius in pixels; the fragment shader measures from F, (|q|^2 + 2 r q.u) / (|q + r u|
// + r) for q = pixel - F, which stays exact in f32 for any radius.  The quad of a piece lies between the chord at the
// inner radius and the tangent at the outer one (over a circle much larger than the target: only the part over it),
// so the pixels it shades follow the stroke.  A pixel near a joint of two pieces is the one's on whose side of the
// joint's radial line its centre lies (both from the same bits), and that one takes the whole ring there.
//
// Dash patterns (common.wgsl's head): the vertex shader finds the pattern's place at a reference point (a segment: the
// camera centre's foot on its line, kept within it; an arc: F kept within the piece) from the nearer end, in
// double-f32; the fragment shader walks the pattern from there, in dash units relative to that place (so that a period
// of a million pixels keeps 1/16 px), and adds each dash that meets the pixel as a piece: the strip between its two ends
// (a square cap half a width further, a round one a half circle), at a join the neighbour's dashes and the join's piece
// where a dash runs through the vertex.  Patterns whose period comes near a pixel turn into a flat tone of their duty
// (DASH_FLAT, DASH_EXACT).
//
// Colours are sRGB codes blended as they are (premultiplied), as lib/gfx blends them; a transparency (the entity's, its
// layer's or its insert's, G6b2) is multiplied into the colour's alpha here (block.wgsl ink).
//
// Blocks (G6b2, block.wgsl): a visible record of an insert comes as ITEM_BLOCK | its place in the pool, its instance
// beside it in vinst; it is placed through its instance's space (a View of the block's units) by the same code as every record, so that both
// owners of a pixel compute it from the same bits.

const BATCH: u32 = 16384u;
const OPEN: u32 = 0u;
const CAP: u32 = 1u;
const JOIN: u32 = 2u;
// segments up to this long (px) and strokes up to this half width (px) take the fast way near their ends
const TINY_LEN: f32 = 3.0;
const TINY_H: f32 = 0.75;
// a dash pattern of a period up to DASH_FLAT pixels is drawn as a flat tone of its duty, from DASH_EXACT on exactly,
// mixed in between
const DASH_FLAT: f32 = 2.0;
const DASH_EXACT: f32 = 4.0;
// beyond this many pixels of arc from a piece's nearer end, the pattern's place at the reference point is computed from
// the chord in double-f32
const ARC_FAR: f32 = 100000.0;
// misc.x of VsOut: start kind | end kind << 2 | cap << 4 | join << 8 | the polyline's ends << 12 (an arc: its own
// ends), and these
const M_DASHED: u32 = 16384u;
const M_RESTART_S: u32 = 32768u;
const M_RESTART_E: u32 = 65536u;
const M_ARC: u32 = 131072u;

struct Push {
    // the visible layer this draw shows (Basis; 0 on Voll, where first_instance carries it in its high 16 bits)
    layer: u32,
}

@group(0) @binding(0) var<uniform> view: View;
@group(0) @binding(1) var<storage, read> pool: array<Record>;
@group(0) @binding(2) var<storage, read> styles: array<Style>;
@group(0) @binding(3) var<storage, read> visible: array<u32>;
@group(0) @binding(4) var<storage, read> lists: array<vec4u>;
// per visible record of an insert its instance (cull.wgsl write); per instance its space this frame
@group(0) @binding(7) var<storage, read> vinst: array<u32>;
@group(0) @binding(8) var<storage, read> spaces: array<Space>;
var<immediate> pc: Push;

struct VsOut {
    @builtin(position) pos: vec4f,
    // a segment: its ends in pixels of the target (clipped where they lay beyond it); an arc's piece: F (px of the
    // target) and the outward unit there
    @location(0) @interpolate(flat) seg: vec4f,
    // a segment, in pixels: the previous point minus the start, the next point minus the end; an arc's piece: its start
    // and end (px of the target)
    @location(1) @interpolate(flat) nb: vec4f,
    // half width (px); a segment: the miter limit, the end minus the start in pixels (from Record.v, as the neighbours
    // have it); an arc's piece: the radius (px), the turn's sign in pixels, the dash reference's angle from F
    @location(2) @interpolate(flat) par: vec4f,
    // flags (see M_*), colour, style
    @location(3) @interpolate(flat) misc: vec4u,
    // a dashed stroke: the pattern's place at the reference point (dash units, hi and lo); a segment: where that point
    // lies (px along it from its drawn start); the place of the other side where the pattern starts again (a closed
    // polyline's first point, a circle's start; dash units)
    @location(4) @interpolate(flat) dash: vec4f,
}

// a visible record (cull.wgsl write): its place in the pool, the view or its insert's space (block.wgsl), whether it is an
// insert's, the style its entities take by block, mirrored, its entry's epx offset, its first hatch row
struct Item {
    idx: u32,
    vw: View,
    blk: bool,
    ins: u32,
    mir: bool,
    eoff: u32,
    hb: u32,
}

// the visible record at place j of the visible list: v its word there (an insert's instance beside it in vinst)
fn item_of(v: u32, j: u32) -> Item {
    var it: Item;
    it.idx = v;
    it.vw = view;
    it.blk = false;
    it.ins = 0u;
    it.mir = false;
    it.eoff = 0u;
    it.hb = 0u;
    if (v & ITEM_BLOCK) != 0u {
        let sp = spaces[vinst[j]];
        it.idx = v & 0x7fffffffu;
        it.vw = space_view(sp);
        it.blk = true;
        it.ins = sp.u.y;
        it.mir = (sp.u.x & SP_MIRROR) != 0u;
        it.eoff = sp.u.w;
        it.hb = sp.u.z;
    }
    return it;
}

// a record's style (block.wgsl style_in); a pattern in pixels inside an insert: its periods per block unit
fn item_style(r: Record, it: Item) -> Style {
    var st = style_in(r.w.z - 1u, it.ins, it.blk);
    if it.blk && dash_count(st) > 0u && ((st.flags >> 20u) & 15u) != UNIT_DRAWING {
        st.kappa = kappa_scaled(st.kappa, it.vw.turn.xy);
    }
    return st;
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

// a dashed segment: the pattern's place at its reference point (the camera centre's foot on its line, kept within the
// segment) from the nearer end, in double-f32 (dash units); where that point lies along the segment from its drawn
// start q0 (px); the place of the other side at a closed polyline's first point.  r0, r1: rel2 of its ends, d its
// direction in pixels; vw: the view or the insert's space (an insert's lengths and its pattern's periods are its block's units)
fn seg_dash(idx: u32, r: Record, st: Style, r0: vec4f, r1: vec4f, q0: vec2f, d: vec2f, vw: View) -> vec4f {
    let ownu = bitcast<vec2f>(r.v.xy);
    let le = length_of(r);
    var ls = vec3f(0.0);
    if (r.w.w & F_FIRST) == 0u {
        ls = length_of(pool[idx - 1u]);
    }
    let lu = length(ownu);
    var du = vec2f(1.0, 0.0);
    if lu > 0.0 {
        du = ownu / lu;
    }
    // the foot's distance from the start along the segment (drawing units): (camera - start) . du
    let ta = dd_add(dd_mul_f(vec2f(-r0.x, -r0.z), du.x), dd_mul_f(vec2f(-r0.y, -r0.w), du.y));
    var base = ls;
    var t = vec2f(0.0);
    var rp = to_px(r0.xy, vw);
    if ta.x >= lu {
        base = le;
        rp = to_px(r1.xy, vw);
    } else if ta.x > 0.0 {
        // (the foot itself: near the camera, a small number out of large ones)
        let fx = dd_add(vec2f(r0.x, r0.z), dd_mul_f(ta, du.x));
        let fy = dd_add(vec2f(r0.y, r0.w), dd_mul_f(ta, du.y));
        rp = to_px(vec2f(fx.x + fx.y, fy.x + fy.y), vw);
        if ta.x <= 0.5 * lu {
            t = ta;
        } else {
            base = le;
            t = dd_add(dd_mul_f(vec2f(-r1.x, -r1.z), du.x), dd_mul_f(vec2f(-r1.y, -r1.w), du.y));
        }
    }
    let ph = dd_mul_f(periods(base, t, st.kappa, st.phase0), st.period);
    // the other side of a closed polyline's first point: its last record ends there, the length all around
    var other = 0.0;
    if (r.w.w & F_WRAP_PREV) != 0u {
        other = periods(length_of(pool[idx + r.w.y]), vec2f(0.0), st.kappa, st.phase0).x * st.period;
    } else if (r.w.w & F_WRAP_NEXT) != 0u {
        other = st.phase0 * st.period;
    }
    return vec4f(ph.x, ph.y, dot(rp - q0, d), other);
}

// the vertex shaders of both pipelines (lib.rs): vs for solid segments only, vs_full also for arcs and dash patterns
// (apart, so that their code weighs nothing on the many solid segments)
@vertex
fn vs(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> VsOut {
    let l = lists[pc.layer + (ii >> 16u)];
    let j = (ii & 0xffffu) * BATCH + (vi >> 2u);
    if j >= l.y {
        return empty_out();
    }
    return vertex(vi, item_of(visible[l.x + j], l.x + j), l.z, false);
}

@vertex
fn vs_full(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> VsOut {
    let l = lists[pc.layer + (ii >> 16u)];
    let j = (ii & 0xffffu) * BATCH + (vi >> 2u);
    if j >= l.y {
        return empty_out();
    }
    return vertex(vi, item_of(visible[l.x + j], l.x + j), l.z, true);
}

// a quad of nothing: all four corners in one point
fn empty_out() -> VsOut {
    var o: VsOut;
    o.pos = vec4f(2.0, 2.0, 0.5, 1.0);
    o.seg = vec4f(0.0);
    o.nb = vec4f(0.0);
    o.par = vec4f(0.0);
    o.misc = vec4u(0u);
    o.dash = vec4f(0.0);
    return o;
}

// a visible record's quad (it: the record, an insert's through its space; layer_t: its layer's transparency)
fn vertex(vi: u32, it: Item, layer_t: u32, full: bool) -> VsOut {
    var o = empty_out();
    let idx = it.idx;
    let vw = it.vw;
    let r = pool[idx];
    let st = item_style(r, it);
    if kind_of_record(r) == K_ARC {
        // (a layer with arcs takes vs_full)
        if full {
            return vs_arc(vi, r, st, it, layer_t);
        }
        return o;
    }
    let h = half_width(st, vw);
    let half = 0.5 * vw.turn.zw;
    let r0 = rel2(r.a, vw);
    let r1 = rel2(r.b, vw);
    // pixels from the target's middle
    let p0 = to_px(r0.xy, vw);
    let p1 = to_px(r1.xy, vw);
    let cap = cap_of(st);
    let info = r.w.w;
    // the neighbours' own vectors: the previous point minus the start, the next point minus the end (scene.rs: the bits
    // of this record's own vector where they meet)
    var nu = vec4f(0.0);
    if (info & F_PREV) != 0u {
        nu = vec4f(-bitcast<vec2f>(pool[select(idx - 1u, idx + r.w.y, (info & F_WRAP_PREV) != 0u)].v.xy), 0.0, 0.0);
    }
    if (info & F_NEXT) != 0u {
        nu = vec4f(nu.xy, bitcast<vec2f>(pool[select(idx + 1u, idx - r.w.y, (info & F_WRAP_NEXT) != 0u)].v.xy));
    }
    var sk = select(CAP, JOIN, (info & F_PREV) != 0u);
    var ek = select(CAP, JOIN, (info & F_NEXT) != 0u);
    let va = to_px(nu.xy, vw);
    let vb = to_px(nu.zw, vw);
    let own = to_px(bitcast<vec2f>(r.v.xy), vw);
    let len0 = length(p1 - p0);
    var d = unit_or_zero(own);
    if !any(d != vec2f(0.0)) {
        d = vec2f(1.0, 0.0);
    }
    // a point without a cap: nothing
    if len0 == 0.0 && cap == CAP_BUTT && sk == CAP && ek == CAP {
        return o;
    }
    let dashed = full && dash_count(st) > 0u;
    var e0 = end_reach(-d, unit_or_zero(va), sk == JOIN, false, h, st);
    var e1 = end_reach(d, unit_or_zero(vb), ek == JOIN, true, h, st);
    if dashed {
        // (a dash's cap near a vertex reaches into the neighbour's room; the pixel's owner draws it, its quad holds it)
        let ce = select(h, 0.0, cap == CAP_BUTT);
        e0 = max(e0, ce);
        e1 = max(e1, ce);
    }
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
        base = far_foot(r0, r1, vw);
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
    // (misc.z: the style whose pattern the fragment shader walks; misc.w: dash units per pixel of a pattern in units, the
    // block's inside an insert)
    o.misc = vec4u(sk | (ek << 2u) | (cap << 4u) | (join_of(st) << 8u) | ((info & 3u) << 12u), ink(st.color, st.r1.x, layer_t), dash_style(r.w.z - 1u, it.ins, it.blk),
                   bitcast<u32>(vw.rest.w));
    if dashed {
        o.misc.x |= M_DASHED | select(0u, M_RESTART_S, (info & F_WRAP_PREV) != 0u) | select(0u, M_RESTART_E, (info & F_WRAP_NEXT) != 0u);
        o.dash = seg_dash(idx, r, st, r0, r1, q0, d, vw);
    }
    return o;
}

// the arc length (drawing units, double-f32) from a point x of the circle (rel2) to F (relative to the camera's
// centre, drawing units), along a circle of radius rr (hi, lo): 2 r asin(c / 2r) of the chord c, asin by one Newton
// step from f32 with the sine in double-f32 (the half angle stays below 0.1: a piece's half)
fn arc_from(x: vec4f, fu: vec2f, rr: vec2f) -> vec2f {
    let cx = dd_add_f(vec2f(-x.x, -x.z), fu.x);
    let cy = dd_add_f(vec2f(-x.y, -x.w), fu.y);
    let c = dd_sqrt(dd_add(dd_mul(cx, cx), dd_mul(cy, cy)));
    let y = dd_div(c, dd_mul_f(rr, 2.0));
    // (asin by its series: the GPU's own asin and cos need not be exact, Vulkan allows 4096 ulp and 2^-11)
    let y2 = y.x * y.x;
    let t0 = y.x * (1.0 + y2 * (1.0 / 6.0 + y2 * (3.0 / 40.0 + y2 * (5.0 / 112.0))));
    let t2 = two_prod(t0, t0);
    let t3 = dd_mul_f(t2, t0);
    let t5 = dd_mul(t3, t2);
    var s = dd_add(dd_mul(t3, vec2f(-0.1666666716337204, 4.967053879312289e-09)), dd_mul(t5, vec2f(0.008333333767950535, -4.34617203337595e-10)));
    s = dd_add_f(s, t5.x * t2.x * (-1.0 / 5040.0 + t2.x / 362880.0));
    s = dd_add_f(s, t0);
    let th = dd_add_f(vec2f(t0, 0.0), ((y.x - s.x) + (y.y - s.y)) / cos_p(t0));
    return dd_mul(dd_mul_f(th, 2.0), rr);
}

// sine and cosine exact also for small angles, which a huge circle's are (the GPU's own may be off by 2^-11 absolute,
// Vulkan's bound, a million pixels at a radius of 1e13 px): their series below 0.5, beyond the GPU's
fn sin_p(x: f32) -> f32 {
    if abs(x) < 0.5 {
        let x2 = x * x;
        return x * (1.0 - x2 / 6.0 * (1.0 - x2 / 20.0 * (1.0 - x2 / 42.0 * (1.0 - x2 / 72.0))));
    }
    return sin(x);
}

// atan2 exact also for the small angles of a huge circle (the GPU's own may be 4096 ulp off): the series of y / x below
// a tenth
fn atan2_p(y: f32, x: f32) -> f32 {
    if x > 0.0 && abs(y) < 0.1 * x {
        let z = y / x;
        let z2 = z * z;
        return z * (1.0 - z2 * (1.0 / 3.0 - z2 * (1.0 / 5.0 - z2 * (1.0 / 7.0 - z2 / 9.0))));
    }
    return atan2(y, x);
}

fn cos_p(x: f32) -> f32 {
    if abs(x) < 0.5 {
        let x2 = x * x;
        return 1.0 - x2 / 2.0 * (1.0 - x2 / 12.0 * (1.0 - x2 / 30.0 * (1.0 - x2 / 56.0)));
    }
    return cos(x);
}

// an arc's piece (the head above): its quad, F and the radius, its ends, its dash pattern's place; it: the record (an
// insert's through its space: a mirrored one turns the other way), layer_t: its layer's transparency
fn vs_arc(vi: u32, r: Record, st: Style, it: Item, layer_t: u32) -> VsOut {
    let idx = it.idx;
    let vw = it.vw;
    var o: VsOut;
    o.pos = vec4f(2.0, 2.0, 0.5, 1.0);
    o.seg = vec4f(0.0);
    o.nb = vec4f(0.0);
    o.par = vec4f(0.0);
    o.misc = vec4u(0u);
    o.dash = vec4f(0.0);
    let h = half_width(st, vw);
    let half = 0.5 * vw.turn.zw;
    let cap = cap_of(st);
    let info = r.w.w;
    let pa = pool[idx - 1u];
    let rr = vec2f(bitcast<f32>(r.v.x), bitcast<f32>(r.v.y));
    let rc = rel2(r.a, vw);
    // the camera's distance from the circle: (|v|^2 - r^2) / (|v| + r) for v = camera - centre, the difference of the
    // squares in double-f32 (both may be 1e14 and more, the distance a thousandth)
    let vx = vec2f(-rc.x, -rc.z);
    let vy = vec2f(-rc.y, -rc.w);
    let v2 = dd_add(dd_mul(vx, vx), dd_mul(vy, vy));
    let num = dd_add(v2, -dd_mul(rr, rr));
    let vl = sqrt(v2.x);
    let d0 = (num.x + num.y) / (vl + rr.x);
    var uu = vec2f(1.0, 0.0);
    if vl > 0.0 {
        uu = vec2f(vx.x, vy.x) / vl;
    }
    // in pixels: F, the outward unit there, the radius; the turn's sign (y points down here: a counter-clockwise arc
    // turns the other way), the arc's direction at F
    let up = unit_or_zero(to_px(uu, vw));
    let fu = -d0 * uu;
    let f = to_px(fu, vw);
    let rp = (rr.x + rr.y) * vw.rest.z;
    let orient = select(1.0, -1.0, (info & F_CCW) != 0u) * select(1.0, -1.0, it.mir);
    let tf = orient * vec2f(-up.y, up.x);
    let ra = rel2(pa.b, vw);
    let ap = to_px(ra.xy, vw);
    let bp = to_px(rel2(r.b, vw).xy, vw);
    // the piece's ends as angles from F along the arc
    let wa = ap - f;
    let wb = bp - f;
    let ba = atan2_p(dot(wa, tf), rp + dot(wa, up));
    var bb = atan2_p(dot(wb, tf), rp + dot(wb, up));
    if bb < ba {
        bb += 6.2831853;
    }
    // a cap's reach beyond the arc's own ends
    let ce = select(h, 0.0, cap == CAP_BUTT);
    var b0 = ba;
    var b1 = bb;
    let mg = half + vec2f(h + ce + 3.0);
    if rp > length(mg) + h + 1.0 {
        // a circle larger than the target and its margin: only the angles at which its ring can meet them (whatever of it
        // lies there is within a quarter turn of F, the point nearest the target's middle: asin of the projection along
        // the arc's direction at F)
        var xmin = 3.4e38;
        var xmax = -3.4e38;
        for (var k = 0u; k < 4u; k++) {
            let x = dot(vec2f(select(-mg.x, mg.x, (k & 1u) != 0u), select(-mg.y, mg.y, (k & 2u) != 0u)) - f, tf);
            xmin = min(xmin, x);
            xmax = max(xmax, x);
        }
        let ri = max(rp - h - 1.0, 1.0);
        let ro = rp + h + 1.0;
        b0 = max(b0, min(asin(clamp(xmin / ri, -1.0, 1.0)), asin(clamp(xmin / ro, -1.0, 1.0))));
        b1 = min(b1, max(asin(clamp(xmax / ri, -1.0, 1.0)), asin(clamp(xmax / ro, -1.0, 1.0))));
        if b0 > b1 {
            return o;
        }
    }
    // the quad: from the chord at the inner radius to the tangent at the outer one, pushed out along the arc by a pixel
    // at its ends (by a cap's reach at the arc's own ends): radial offsets from the circle, exact for any radius
    let al = 0.5 * (b1 - b0);
    let sal = sin_p(0.5 * al);
    let dout = ((h + 1.0) + 2.0 * rp * sal * sal) / cos_p(al);
    let din = -min(h + 1.0, rp);
    let e0 = select(1.0, 1.0 + ce, (info & F_CAP_A) != 0u && b0 == ba);
    let e1 = select(1.0, 1.0 + ce, (info & F_CAP_B) != 0u && b1 == bb);
    let be = select(b0, b1, (vi & 1u) != 0u);
    let dr = select(din, dout, (vi & 2u) != 0u);
    let sb = sin_p(be);
    let cb = cos_p(be);
    let sh = sin_p(0.5 * be);
    // a point at angle be and radius rp + dr, relative to F; then along the arc's direction there
    var p = f + (dr * cb - 2.0 * rp * sh * sh) * up + (rp + dr) * sb * tf;
    p += select(-e0, e1, (vi & 1u) != 0u) * (cb * tf - sb * up);
    o.pos = vec4f(p.x / half.x, -p.y / half.y, 0.5, 1.0);
    o.seg = vec4f(f + half, up);
    o.nb = vec4f(ap + half, bp + half);
    o.par = vec4f(h, rp, orient, 0.0);
    o.misc = vec4u(M_ARC | (cap << 4u) | ((info & 3u) << 12u), ink(st.color, st.r1.x, layer_t), dash_style(r.w.z - 1u, it.ins, it.blk), bitcast<u32>(vw.rest.w));
    if dash_count(st) > 0u {
        o.misc.x |= M_DASHED | select(0u, M_RESTART_S, (info & F_RESTART_A) != 0u) | select(0u, M_RESTART_E, (info & F_RESTART_B) != 0u);
        // the reference point: F kept within the piece; the pattern's place there from the nearer end
        let br = clamp(0.0, ba, bb);
        let la = length_of(pa);
        let lb = length_of(r);
        var base = la;
        var t = vec2f(0.0);
        if br >= bb {
            base = lb;
        } else if br > ba {
            // (beyond ARC_FAR pixels f32's angle is too coarse: the chord in double-f32)
            let ru = rr.x + rr.y;
            let near_a = -ba <= bb;
            let ang = select(bb, -ba, near_a);
            if rp * ang > ARC_FAR {
                t = arc_from(select(rel2(r.b, vw), ra, near_a), fu, rr);
            } else {
                t = vec2f(ru * ang, 0.0);
            }
            if !near_a {
                base = lb;
                t = -t;
            }
        }
        let ph = dd_mul_f(periods(base, t, st.kappa, st.phase0), st.period);
        var other = 0.0;
        if (info & F_RESTART_A) != 0u {
            other = periods(length_of(pool[idx + r.w.y]), vec2f(0.0), st.kappa, st.phase0).x * st.period;
        } else if (info & F_RESTART_B) != 0u {
            other = st.phase0 * st.period;
        }
        o.par.w = br;
        o.dash = vec4f(ph.x, ph.y, 0.0, other);
    }
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

// how a dash side's end ends (pieces): open (clipped by the target: no cut), cut (a join where the dash runs on), with
// the style's cap (a dash's end, the polyline's or arc's own end)
const E_OPEN: u32 = 0u;
const E_CUT: u32 = 1u;
const E_CAP: u32 = 2u;

// the dashes of a pattern near a point
struct Ivals {
    n: u32,
    i0: vec2f,
    i1: vec2f,
    i2: vec2f,
}

// where entry j of a style's dash pattern ends, from the period's start (dash units)
// (the fragment shader reads a style's fields from the buffer, by its index: a copy of the struct with its array of
// ends indexed at run time would lie in local memory, and every pixel would pay for it)
fn dash_end(si: u32, j: u32) -> f32 {
    return styles[si].ends[j >> 2u][j & 3u];
}

fn dash_n(si: u32) -> u32 {
    return (styles[si].flags >> 12u) & 31u;
}

// the dashes (on entries) of a style's pattern within w px of a point at its place xh + xl (dash units), as px along
// the stroke from the point (sc dash units per px), at most three.  Relative to the point, so that the ends near it are
// exact also when the period is a million pixels (xh a place in the period, xl a small offset).  From the entry that
// holds the place w px before the point, forward, each entry's start carried along: no index j - 1 and no step back.
// (Stepping back from the point's entry as j = select(j - 1u, n - 1u, j == 0u), then reading the entry's length with a
// test j == 0, Mesa's Intel compiler (26.0, HD 610 and Iris Xe) took that test for the step's own j == 0: a dash
// shorter than its gap lost the pixels it covers whole, docs/RENDER2D.md)
fn ivals(si: u32, xh: f32, xl: f32, sc: f32, w: f32) -> Ivals {
    var o: Ivals;
    o.n = 0u;
    o.i0 = vec2f(0.0);
    o.i1 = vec2f(0.0);
    o.i2 = vec2f(0.0);
    let n = dash_n(si);
    let p = styles[si].period;
    let kp = floor((xh + xl) / p);
    let x0 = xh - kp * p;
    let wd = w * sc;
    // the entry that holds the place wd before the point: the first whose end lies beyond it, in the period before when
    // the period starts after it; compared relative to the point (the place x0 + xl itself is rounded to the period's
    // size), and where that entry starts in the period
    var back = 0.0;
    if (0.0 - x0) - xl > -wd {
        back = p;
    }
    var j = 0u;
    var cs = 0.0;
    loop {
        if j + 1u >= n {
            break;
        }
        let c1 = dash_end(si, j);
        if ((c1 - back) - x0) - xl > -wd {
            break;
        }
        cs = c1;
        j += 1u;
    }
    var start = ((cs - back) - x0) - xl;
    var g = 0u;
    loop {
        if start > wd || g >= 32u || o.n >= 3u {
            break;
        }
        let ce = dash_end(si, j);
        let e = ce - cs;
        if (j & 1u) == 0u && start + e >= -wd {
            let v = vec2f(start, start + e) / sc;
            if o.n == 0u {
                o.i0 = v;
            } else if o.n == 1u {
                o.i1 = v;
            } else {
                o.i2 = v;
            }
            o.n += 1u;
        }
        start += e;
        cs = ce;
        j += 1u;
        if j >= n {
            j = 0u;
            cs = 0.0;
        }
        g += 1u;
    }
    return o;
}

// the pattern's entry that holds its place xh + xl: how far it reaches back and forward from there (px; sc dash units
// per px), exact near its ends as in ivals
fn entry_span(si: u32, xh: f32, xl: f32, sc: f32) -> vec2f {
    let n = dash_n(si);
    let p = styles[si].period;
    let kp = floor((xh + xl) / p);
    let x0 = xh - kp * p;
    let x = x0 + xl;
    var j = 0u;
    var c0 = 0.0;
    var c1 = dash_end(si, 0u);
    loop {
        if j + 1u >= n || x < c1 {
            break;
        }
        j += 1u;
        c0 = c1;
        c1 = dash_end(si, j);
    }
    return vec2f(-((c0 - x0) - xl), (c1 - x0) - xl) / sc;
}

// the pattern is on at its place xh + xl
fn on_at(si: u32, xh: f32, xl: f32) -> bool {
    let n = dash_n(si);
    let p = styles[si].period;
    let kp = floor((xh + xl) / p);
    let x = (xh - kp * p) + xl;
    var j = 0u;
    loop {
        if j + 1u >= n {
            break;
        }
        if x < dash_end(si, j) {
            break;
        }
        j += 1u;
    }
    return (j & 1u) == 0u;
}

// no dash (clip_iv): its start beyond its end
const IV_NONE: vec4f = vec4f(1e30, -1e30, 0.0, 0.0);

// a dash (px along its side, relative to the side's origin) clipped to the side's extent lim (ends k: E_*): (z0, z1, how
// each end ends: 0 open, 1 cut, 2 the style's cap; an end within 1/1000 px of the extent's: there); none: IV_NONE
fn clip_iv(v: vec2f, uq: f32, lim: vec2f, k: vec2u) -> vec4f {
    var z0 = v.x + uq;
    var z1 = v.y + uq;
    if z1 < lim.x || z0 > lim.y {
        return IV_NONE;
    }
    var a = 2.0;
    var b = 2.0;
    if z0 <= lim.x + 1e-3 {
        z0 = select(lim.x, z0, k.x == E_OPEN);
        a = select(select(1.0, 2.0, k.x == E_CAP), 0.0, k.x == E_OPEN);
    }
    if z1 >= lim.y - 1e-3 {
        z1 = select(lim.y, z1, k.y == E_OPEN);
        b = select(select(1.0, 2.0, k.y == E_CAP), 0.0, k.y == E_OPEN);
    }
    return vec4f(z0, z1, a, b);
}

// the geometry a dashed stroke's pieces are cut from, at the pixel q: a segment side (origin o, direction dd) or the ring
// of an arc (at the pixel: its outward unit m, its direction t, the pixel's distance from the circle, the radius)
struct Ring {
    m: vec2f,
    t: vec2f,
    dist: f32,
    r: f32,
}

// a point of a side at z px along it (relative to its origin; an arc: arc length from the pixel's own place) and the
// side's direction there (x, y: the point; z, w: the direction)
fn side_at(q: vec2f, arc: bool, o: vec2f, dd: vec2f, g: Ring, z: f32) -> vec4f {
    if !arc {
        return vec4f(o + z * dd, dd);
    }
    let a = z / g.r;
    let sa = sin_p(a);
    let sh = sin_p(0.5 * a);
    let ca = cos_p(a);
    return vec4f(q + (-2.0 * g.r * sh * sh - g.dist) * g.m + g.r * sa * g.t, ca * g.t - sa * g.m);
}

// a dashed segment's or an arc's coverage of the pixel q, its owner decided (fs): the dashes of each side that meet the
// pixel as pieces (the strip between the dash's ends, cut by a square cap half a width further, a round cap's half
// circle a piece of its own, round caps of neighbouring dashes that meet taken once: |a| + |b| - |a and b|), near a join
// the neighbour's dashes and the join's piece where a dash runs through the vertex, the rectangles meeting there taken
// away (the union of docs/RENDER2D.md).  Sides: 0 this segment or the arc, 1 the previous segment (an arc: the other side
// where the pattern starts again at its start), 2 the next one (at its end), 3 the joins.  solid: one dash all along
// (an arc's own caps; the flat tone)
fn pieces(i: VsOut, q: vec2f, arc: bool, solid: bool, si: u32, sc: f32, d: vec2f, dp: vec2f, dn: vec2f, near_s: bool, near_e: bool, g: Ring) -> f32 {
    let fl = i.misc.x;
    let h = i.par.x;
    let cap = (fl >> 4u) & 15u;
    let sk = fl & 3u;
    let ek = (fl >> 2u) & 3u;
    let jk = (fl >> 8u) & 15u;
    let cp = (fl & 4096u) != 0u;
    let cn = (fl & 8192u) != 0u;
    let rs = (fl & M_RESTART_S) != 0u;
    let re = (fl & M_RESTART_E) != 0u;
    let w = 0.71 + select(h, 0.0, cap == CAP_BUTT);
    // this side's own extent and the pattern along it: a segment from s (its place at s: xh + xl), an arc from the pixel's
    // own place on it (z = 0) to its ends A and B
    var s = i.seg.xy;
    var e = i.seg.zw;
    var lens = length(e - s);
    var xh = i.dash.x;
    var xl = i.dash.y - i.dash.z * sc;
    var za = 0.0;
    var zb = lens;
    var ka = select(select(E_CAP, E_CUT, sk == JOIN), E_OPEN, sk == OPEN);
    var kb = select(select(E_CAP, E_CUT, ek == JOIN), E_OPEN, ek == OPEN);
    if arc {
        let f = i.seg.xy;
        let up = i.seg.zw;
        let rp = i.par.y;
        let orient = i.par.z;
        let wv = (q - f) + rp * up;
        let da = unit_or_zero((i.nb.xy - f) + rp * up);
        let db = unit_or_zero((i.nb.zw - f) + rp * up);
        let ta = orient * vec2f(-da.y, da.x);
        let tb = orient * vec2f(-db.y, db.x);
        // the arc lengths from the pixel's place to A and to B, and from the dash reference to the pixel's place
        za = -rp * atan2_p(dot(wv, ta), dot(wv, da));
        zb = -rp * atan2_p(dot(wv, tb), dot(wv, db));
        let br = i.par.w;
        let cr = cos_p(br);
        let sr = sin_p(br);
        let tf = orient * vec2f(-up.y, up.x);
        let dr = cr * up + sr * tf;
        let tr = cr * tf - sr * up;
        let srp = rp * atan2_p(dot(q - f, tr) - rp * sr, dot(q - f, dr) + rp * cr);
        xh = i.dash.x;
        xl = i.dash.y + srp * sc;
        let capa = cp;
        let capb = cn;
        ka = select(select(E_OPEN, E_CUT, rs), E_CAP, capa);
        kb = select(select(E_OPEN, E_CUT, re), E_CAP, capb);
        s = q;
    }
    // on at the ends: this side's place, the other side's (the same unless the pattern starts again there); a cut end
    // whose other side is off takes a cap
    let on_s = solid || on_at(si, xh, xl + za * sc);
    let on_e = solid || on_at(si, xh, xl + zb * sc);
    let ps_h = select(xh, i.dash.w, rs);
    let ps_l = select(xl + za * sc, 0.0, rs);
    let ne_h = select(xh, i.dash.w, re);
    let ne_l = select(xl + zb * sc, 0.0, re);
    let pon_s = solid || on_at(si, ps_h, ps_l);
    let non_e = solid || on_at(si, ne_h, ne_l);
    if ka == E_CUT && !pon_s {
        ka = E_CAP;
    }
    if kb == E_CUT && !non_e {
        kb = E_CAP;
    }
    let lp = length(i.nb.xy);
    let ln = length(i.nb.zw);
    let joins_s = !arc && near_s && sk == JOIN && on_s && pon_s;
    let joins_e = !arc && near_e && ek == JOIN && on_e && non_e;
    // the sides that may hold pieces, as bits (loops over bits, not over a count the compiler could unroll)
    var sides = 1u;
    if arc {
        sides |= select(0u, 2u, rs && !solid) | select(0u, 4u, re && !solid);
    } else {
        sides |= select(0u, 2u, near_s && sk == JOIN) | select(0u, 4u, near_e && ek == JOIN) | select(0u, 8u, joins_s || joins_e);
    }
    var c = 0.0;
    // how far the dashes that run through a join reach from the vertex, within their segments (the joins' overlap is cut
    // there): this segment's from S and to E, the previous one's to S, the next one's from E; solid: the segments (the
    // neighbours' far ends open as above)
    var ls_own = lens;
    var le_own = lens;
    var ls_nb = select(1e30, lp, cp || near_e);
    var le_nb = select(1e30, ln, cn || near_s);
    if !solid && !arc {
        let sq = select(0.0, h, cap == CAP_SQUARE);
        if joins_s {
            ls_own = min(entry_span(si, xh, xl, sc).y + sq, lens);
            ls_nb = min(entry_span(si, ps_h, ps_l, sc).x + sq, lp);
        }
        if joins_e {
            le_own = min(entry_span(si, xh, xl + zb * sc, sc).x + sq, lens);
            le_nb = min(entry_span(si, ne_h, ne_l, sc).y + sq, ln);
        }
    }
    loop {
        if sides == 0u {
            break;
        }
        let side = firstTrailingBit(sides);
        sides &= sides - 1u;
        // the side: origin, direction, the pattern's place at the origin, its extent and how its ends end
        var o = s;
        var dd = d;
        var ph = xh;
        var pl = xl;
        var lim = vec2f(select(za, -1e30, ka == E_OPEN), select(zb, 1e30, kb == E_OPEN));
        var k = vec2u(ka, kb);
        if side == 1u {
            if arc {
                // the other side at A where the pattern starts again: it runs up to A
                ph = ps_h;
                pl = ps_l - za * sc;
                lim = vec2f(-1e30, za);
                k = vec2u(E_OPEN, select(E_CAP, E_CUT, on_s));
            } else {
                dd = dp;
                ph = ps_h;
                pl = ps_l;
                lim = vec2f(-lp, 0.0);
                k = vec2u(select(select(E_OPEN, E_CUT, near_e), E_CAP, cp), select(E_CAP, E_CUT, on_s));
                lim.x = select(-lp, -1e30, k.x == E_OPEN);
            }
        } else if side == 2u {
            if arc {
                ph = ne_h;
                pl = ne_l - zb * sc;
                lim = vec2f(zb, 1e30);
                k = vec2u(select(E_CAP, E_CUT, on_e), E_OPEN);
            } else {
                o = e;
                dd = dn;
                ph = ne_h;
                pl = ne_l;
                lim = vec2f(0.0, ln);
                k = vec2u(select(E_CAP, E_CUT, on_e), select(select(E_OPEN, E_CUT, near_s), E_CAP, cn));
                lim.y = select(ln, 1e30, k.y == E_OPEN);
            }
        }
        // the side's dashes near the pixel, clipped to its extent; square caps lengthen them and dashes that then meet
        // become one
        let uq = select(dot(q - o, dd), 0.0, arc);
        var iv: Ivals;
        if solid {
            iv.n = 1u;
            iv.i0 = vec2f(-1e30, 1e30);
        } else if side < 3u {
            iv = ivals(si, ph, pl + uq * sc, sc, w);
        }
        var v0 = clip_iv(iv.i0, uq, lim, k);
        var v1 = clip_iv(iv.i1, uq, lim, k);
        var v2 = clip_iv(iv.i2, uq, lim, k);
        let nv = select(iv.n, 0u, side == 3u);
        if nv < 3u {
            v2 = IV_NONE;
        }
        if nv < 2u {
            v1 = IV_NONE;
        }
        if nv < 1u {
            v0 = IV_NONE;
        }
        // (a segment's square caps lengthen its strip; an arc's are rectangles along its direction at the dash's end: pieces
        // of their own, as round caps)
        if cap == CAP_SQUARE && !arc {
            v0 = select(v0, vec4f(v0.x - h * f32(v0.z == 2.0), v0.y + h * f32(v0.w == 2.0), min(v0.z, 1.0), min(v0.w, 1.0)), v0.x <= v0.y);
            v1 = select(v1, vec4f(v1.x - h * f32(v1.z == 2.0), v1.y + h * f32(v1.w == 2.0), min(v1.z, 1.0), min(v1.w, 1.0)), v1.x <= v1.y);
            v2 = select(v2, vec4f(v2.x - h * f32(v2.z == 2.0), v2.y + h * f32(v2.w == 2.0), min(v2.z, 1.0), min(v2.w, 1.0)), v2.x <= v2.y);
            if v1.x <= v1.y && v2.x <= v2.y && v1.y >= v2.x {
                v1 = vec4f(v1.x, v2.y, v1.z, v2.w);
                v2 = IV_NONE;
            }
            if v0.x <= v0.y && v1.x <= v1.y && v0.y >= v1.x {
                v0 = vec4f(v0.x, v1.y, v0.z, v1.w);
                v1 = v2;
                v2 = IV_NONE;
            }
        }
        let round = cap == CAP_ROUND || (cap == CAP_SQUARE && arc);
        let square = cap == CAP_SQUARE;
        // the pieces as bits: per dash j bits 3j (its strip), 3j + 1 and 3j + 2 (its caps as pieces: round, an arc's square);
        // per pair of neighbouring dashes whose caps meet bits 9 + 3j (the caps), 10 + 3j and 11 + 3j (a cap and the other
        // dash: taken away); side 3: the join's piece and the rectangles meeting at the start (bits 0, 1) and at the end (2, 3)
        var pm = 0u;
        if side == 3u {
            pm = select(0u, 3u, joins_s) | select(0u, 12u, joins_e);
        } else {
            pm = select(0u, 1u, v0.x <= v0.y) | select(0u, 8u, v1.x <= v1.y) | select(0u, 64u, v2.x <= v2.y);
            if round {
                pm |= select(0u, 2u, v0.x <= v0.y && v0.z == 2.0) | select(0u, 4u, v0.x <= v0.y && v0.w == 2.0);
                pm |= select(0u, 16u, v1.x <= v1.y && v1.z == 2.0) | select(0u, 32u, v1.x <= v1.y && v1.w == 2.0);
                pm |= select(0u, 128u, v2.x <= v2.y && v2.z == 2.0) | select(0u, 256u, v2.x <= v2.y && v2.w == 2.0);
                let g01 = v1.x - v0.y;
                if v0.x <= v0.y && v1.x <= v1.y && v0.w == 2.0 && v1.z == 2.0 && g01 < 2.0 * h {
                    pm |= select(512u, 3584u, g01 < h);
                }
                let g12 = v2.x - v1.y;
                if v1.x <= v1.y && v2.x <= v2.y && v1.w == 2.0 && v2.z == 2.0 && g12 < 2.0 * h {
                    pm |= select(4096u, 28672u, g12 < h);
                }
            }
        }
        var stl = strip_l(q, o, dd, h);
        var str = strip_r(q, o, dd, h);
        if arc {
            stl = vec3f(g.m, g.dist - h);
            str = select(ALL, vec3f(-g.m, -g.dist - h), g.r > h);
        }
        loop {
            if pm == 0u {
                break;
            }
            let kk = firstTrailingBit(pm);
            pm &= pm - 1u;
            var x0 = ALL;
            var x1 = ALL;
            var x2 = ALL;
            var x3 = ALL;
            var x4 = ALL;
            var x5 = ALL;
            var wt = 1.0;
            if side == 3u {
                let at_s = kk < 2u;
                var jj: Join;
                if at_s {
                    jj = make_join(s, dp, d, jk, i.par.y, h);
                } else {
                    jj = make_join(e, d, dn, jk, i.par.y, h);
                }
                if (kk & 1u) == 0u {
                    let wd = wedge(q, jj, h);
                    x0 = wd.a;
                    x1 = wd.b;
                    x2 = wd.c;
                    x3 = wd.d;
                } else {
                    // the rectangles meeting on the inner side of the turn: between the two strips' inner lines and the lines at
                    // right angles through the vertex, cut where the dashes through it end (their outer lines lie beyond)
                    let nbd = select(dn, dp, at_s);
                    let vtx = select(e, s, at_s);
                    let din = select(d, dp, at_s);
                    let dout = select(dn, d, at_s);
                    let left = din.x * dout.y - din.y * dout.x >= 0.0;
                    x0 = select(strip_r(q, s, d, h), strip_l(q, s, d, h), left);
                    x1 = select(strip_r(q, vtx, nbd, h), strip_l(q, vtx, nbd, h), left);
                    x2 = before(q, vtx, din);
                    x3 = before(q, vtx, -dout);
                    let lin = select(le_own, ls_nb, at_s);
                    let lout = select(le_nb, ls_own, at_s);
                    x4 = select(before(q, vtx - lin * din, -din), ALL, lin > 1e29);
                    x5 = select(before(q, vtx + lout * dout, dout), ALL, lout > 1e29);
                    wt = -1.0;
                }
            } else if kk < 9u {
                let iv_k = kk / 3u;
                let v = select(select(v2, v1, iv_k == 1u), v0, iv_k == 0u);
                let sub = kk % 3u;
                if sub == 0u {
                    let pa = side_at(q, arc, o, dd, g, v.x);
                    let pb = side_at(q, arc, o, dd, g, v.y);
                    x0 = stl;
                    x1 = str;
                    x2 = select(ALL, before(q, pa.xy, -pa.zw), v.z > 0.5);
                    x3 = select(ALL, before(q, pb.xy, pb.zw), v.w > 0.5);
                } else {
                    // (a cap beyond the dash's end, outward along ot: a half circle, or a rectangle half a width long)
                    let pc = side_at(q, arc, o, dd, g, select(v.y, v.x, sub == 1u));
                    let ot = select(pc.zw, -pc.zw, sub == 1u);
                    x0 = tangent(q, pc.xy, h);
                    x1 = before(q, pc.xy, -ot);
                    if square {
                        x0 = strip_l(q, pc.xy, ot, h);
                        x2 = strip_r(q, pc.xy, ot, h);
                        x3 = vec3f(ot, dot(q - pc.xy, ot) - h);
                    }
                }
            } else {
                // dashes j and j + 1 whose caps meet: |caps|, |cap of j and dash j + 1|, |dash j and cap of j + 1| (an arc's square
                // caps: their rectangles, both cut by one of their strips, the near cap's: six planes)
                let pj = (kk - 9u) / 3u;
                let va = select(v1, v0, pj == 0u);
                let vb = select(v2, v1, pj == 0u);
                let ea = side_at(q, arc, o, dd, g, va.y);
                let eb = side_at(q, arc, o, dd, g, vb.x);
                let sub = (kk - 9u) % 3u;
                wt = -1.0;
                // (cap j beyond ea along ea.zw, cap j + 1 before eb, against eb.zw)
                let ha0 = select(tangent(q, ea.xy, h), strip_l(q, ea.xy, ea.zw, h), square);
                let ha1 = select(ALL, strip_r(q, ea.xy, ea.zw, h), square);
                let ha2 = before(q, ea.xy, -ea.zw);
                let ha3 = select(ALL, vec3f(ea.zw, dot(q - ea.xy, ea.zw) - h), square);
                let hb0 = select(tangent(q, eb.xy, h), strip_l(q, eb.xy, eb.zw, h), square);
                let hb1 = select(ALL, strip_r(q, eb.xy, eb.zw, h), square);
                let hb2 = before(q, eb.xy, eb.zw);
                let hb3 = select(ALL, vec3f(-eb.zw, dot(q - eb.xy, -eb.zw) - h), square);
                if sub == 0u {
                    x0 = ha0;
                    x1 = ha1;
                    x2 = ha2;
                    x3 = ha3;
                    x4 = select(hb0, hb2, square);
                    x5 = select(hb2, hb3, square);
                } else if sub == 1u {
                    let s_b = side_at(q, arc, o, dd, g, vb.y);
                    x0 = ha0;
                    x1 = ha2;
                    x2 = select(stl, ha1, square);
                    x3 = select(str, ha3, square);
                    x4 = before(q, eb.xy, -eb.zw);
                    x5 = select(ALL, before(q, s_b.xy, s_b.zw), vb.w > 0.5);
                } else {
                    let s_a = side_at(q, arc, o, dd, g, va.x);
                    x0 = select(stl, hb1, square);
                    x1 = select(str, hb3, square);
                    x2 = select(ALL, before(q, s_a.xy, -s_a.zw), va.z > 0.5);
                    x3 = before(q, ea.xy, ea.zw);
                    x4 = hb0;
                    x5 = hb2;
                }
            }
            c += wt * area6(x0, x1, x2, x3, x4, x5);
        }
    }
    return clamp(c, 0.0, 1.0);
}

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
        let col = unpack4x8unorm(i.misc.y);
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
        let col = unpack4x8unorm(i.misc.y);
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
    // (the colour unpacked where it is returned, as in full_body)
    let col = unpack4x8unorm(i.misc.y);
    let al = col.w * c;
    return vec4f(vec3f(col.z, col.y, col.x) * al, al);
}

// the fragment shader of layers that hold arcs or dashed strokes (lib.rs: the second pipeline; fs draws the others): a
// pixel's owner decided as in fs (an arc's pieces at their joints: by the side of the joint's radial line), the
// coverage from pieces(): the pattern exactly, the solid stroke near its ends (the union at joins: area6 has one call
// in this shader, a second one made the driver call it as a function, and every pixel three times slower), mixed where
// the pattern's period nears a pixel.  Solid segments here as in fs except that near their joins the union stands for
// the tiles.  Its body is in full_body.wgsl: lib.rs puts it into the entry fs_full here and once more into the function
// full_body, with which fs_fill draws the strokes of its layers (a discard there, and the caller returns at once).  As a
// function called from fs_full too, the NVIDIA driver made every pixel of the second pipeline 20 % slower
