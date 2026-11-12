// common -- what cull.wgsl and stroke.wgsl share (lib.rs puts it in front of both): the view, records, styles, the
// coordinates relative to the camera, how far a segment's stroke reaches, double-f32 arithmetic and where a dash
// pattern stands (docs/RENDER2D.md)
//
// Coordinates (decision 28): a point is f64 on the CPU and a pair of f32 per axis here, hi = f32(x) and lo = f32(x -
// hi).  The camera's centre comes the same way, and what f32 cannot hold of it (`rest`, at most 2^-26 of a unit at
// 1e7) is taken off afterwards.  rel() subtracts the camera first: hi - hi and lo - lo each through an exact two-sum,
// so a point near the camera comes out to a few 1e-15 units; a point far from it as a double-f32 pair (rel2), whose
// rounded part has f32's 24 bits of its distance.  Pixels are then y down from the target's top left corner.
//
// Dash patterns: a record holds the length along its entity at its end (f64 as three f32, exact), the record before
// it the length at its start.  The pattern's place at a length L is frac(L k + p0) periods, k the periods per drawing
// unit (the style's, from the CPU in f64 as three f32: per frame for patterns in pixels) and p0 the phase; L k can be
// 1e13 and more (1e7 units at 1e-6 units per pixel), beyond f32 and beyond double-f32 products, so `periods` takes
// the exact pieces of the products (two_prod) and adds only their fractions, in double-f32.

struct View {
    // the camera's centre: hi.x, hi.y, lo.x, lo.y (drawing units)
    center: vec4f,
    // what hi + lo leaves of it (x, y), 1 / scale (pixels per unit), scale (units per pixel)
    rest: vec4f,
    // cos and sin of the rotation, the target's width and height (px)
    turn: vec4f,
    // x: 1 when culling is off; y: 0, the opaque zero of opq; the rest unused
    misc: vec4f,
}

// a record, 64 bytes (scene.rs records()).  A segment of a polyline: its start and end; an arc's piece: its circle's
// centre and its end on the circle (its start is the end of the record before it: the arc's head or the piece before)
struct Record {
    // a segment's start, an arc piece's centre: hi.x, hi.y, lo.x, lo.y
    a: vec4f,
    // its end (an arc's head: the arc's start)
    b: vec4f,
    // x, y: a segment's end minus its start as two f32 (the CPU rounded the f64 difference once: exactly the negated
    // vector to the previous point of the next record and the vector to the next point of the one before; the vertex
    // shader reads its neighbours' for a join's bisector); an arc piece: its radius, hi and lo.  z, w: the length along
    // the entity at the end (drawing units), the first two of three f32
    v: vec4u,
    // x: the length's third f32; y: a closed polyline's first and last record: how far the other one lies, a circle's
    // first and last piece likewise; z: style + 1 (0: a free record, an arc's head); w: flags (F_*) | kind << 8
    w: vec4u,
}

const K_SEG: u32 = 0u;
const K_ARC: u32 = 1u;
// a fill's boundary edge (a ring: a closed polyline of them, a start, an end) and triangle (a.xyz: how far back the
// edge records of its corners lie, as u32)
const K_EDGE: u32 = 3u;
const K_TRI: u32 = 4u;
// a segment: its previous point is the polyline's first (a cap there); its next point is the last
const F_CAP_PREV: u32 = 1u;
const F_CAP_NEXT: u32 = 2u;
// a segment joined at its start, at its end
const F_PREV: u32 = 4u;
const F_NEXT: u32 = 8u;
// the first record of its entity: the length along it starts at 0
const F_FIRST: u32 = 16u;
// a closed polyline's first record (the previous one lies w.y records on) and its last (the next one lies w.y back)
const F_WRAP_PREV: u32 = 32u;
const F_WRAP_NEXT: u32 = 64u;
// an arc piece: its start is the arc's start (a cap), its end the arc's end
const F_CAP_A: u32 = 1u;
const F_CAP_B: u32 = 2u;
// the arc turns counter-clockwise (the drawing's y up)
const F_CCW: u32 = 4u;
// a circle's first piece (its start is where the dash pattern starts again) and its last
const F_RESTART_A: u32 = 32u;
const F_RESTART_B: u32 = 64u;

fn kind_of_record(r: Record) -> u32 {
    return (r.w.w >> 8u) & 15u;
}

// the length along the entity at a record's end (three f32)
fn length_of(r: Record) -> vec3f {
    return vec3f(bitcast<f32>(r.v.z), bitcast<f32>(r.v.w), bitcast<f32>(r.w.x));
}

// a style, 128 bytes (scene.rs Style::words)
struct Style {
    // 0xAARRGGBB, sRGB codes, straight alpha
    color: u32,
    width: f32,
    // unit | cap << 4 | join << 8 | dash entries (0 ..= 16, an odd pattern twice) << 12 | the dash unit << 20
    flags: u32,
    // miter limit: the longest miter over the width
    miter: f32,
    // the dash pattern's periods per drawing unit at this frame (three f32), where it starts (periods)
    kappa: vec3f,
    phase0: f32,
    // its period (dash units), the share of a period on (without caps), the dashes per period
    period: f32,
    duty: f32,
    ons: f32,
    // the style's place in the hatch table + 1 (fill.wgsl; 0: no hatch)
    hatch: u32,
    // the entries' ends from the period's start (dash units), 16
    ends: array<vec4f, 4>,
    r1: vec4u,
}

const UNIT_DRAWING: u32 = 1u;
const CAP_BUTT: u32 = 0u;
const CAP_ROUND: u32 = 1u;
const CAP_SQUARE: u32 = 2u;
const JOIN_MITER: u32 = 0u;
const JOIN_ROUND: u32 = 1u;
const JOIN_BEVEL: u32 = 2u;
// records per page: what a workgroup of culling looks at
const PAGE: u32 = 256u;

// x as it is, through a door no compiler sees through (view.misc.y is 0; its bits xor'ed in): the NVIDIA driver
// simplified (a + b) - a to b in a two-sum despite NoContraction, and an error term became 0 (a circle of radius 1e7
// then lay 0.02 units from its place); where a sum or product passes this, no algebra crosses it
fn opq(x: f32) -> f32 {
    return bitcast<f32>(bitcast<u32>(x) ^ bitcast<u32>(view.misc.y));
}

// a + b exactly as s + e (Knuth's two-sum; round to nearest, nothing reassociated, nothing simplified: opq)
fn two_sum(a: f32, b: f32) -> vec2f {
    let s = opq(a + b);
    let bb = opq(s - a);
    let e = (a - opq(s - bb)) + (b - bb);
    return vec2f(s, e);
}

// a + b as s + e when |a| >= |b| (Dekker's fast two-sum)
fn quick_sum(a: f32, b: f32) -> vec2f {
    let s = opq(a + b);
    return vec2f(s, b - opq(s - a));
}

// a * b exactly as p + e: Dekker's product of the halves (12 bits each: their products are exact, a fused
// multiply-add of them changes nothing).  Not fma: SPIR-V's Fma is one rounding only where it carries NoContraction,
// which lib/shader gives the arithmetic operations alone
fn split_f(a: f32) -> vec2f {
    let t = opq(4097.0 * a);
    let hi = t - opq(t - a);
    return vec2f(hi, a - hi);
}

fn two_prod(a: f32, b: f32) -> vec2f {
    let p = opq(a * b);
    let x = split_f(a);
    let y = split_f(b);
    return vec2f(p, ((x.x * y.x - p) + x.x * y.y + x.y * y.x) + x.y * y.y);
}

// double-f32 (hi, lo): sums and products to about 2^-47 (only in the passes compiled exact: lib.rs)
fn dd_add(a: vec2f, b: vec2f) -> vec2f {
    let s = two_sum(a.x, b.x);
    return quick_sum(s.x, s.y + (a.y + b.y));
}

fn dd_add_f(a: vec2f, b: f32) -> vec2f {
    let s = two_sum(a.x, b);
    return quick_sum(s.x, s.y + a.y);
}

fn dd_mul(a: vec2f, b: vec2f) -> vec2f {
    let p = two_prod(a.x, b.x);
    return quick_sum(p.x, p.y + (a.x * b.y + a.y * b.x));
}

fn dd_mul_f(a: vec2f, b: f32) -> vec2f {
    let p = two_prod(a.x, b);
    return quick_sum(p.x, p.y + a.y * b);
}

fn dd_sqrt(a: vec2f) -> vec2f {
    if a.x <= 0.0 {
        return vec2f(0.0);
    }
    let x = sqrt(a.x);
    let p = two_prod(x, x);
    return quick_sum(x, ((a.x - p.x) - p.y + a.y) / (2.0 * x));
}

fn dd_div(a: vec2f, b: vec2f) -> vec2f {
    let q1 = a.x / b.x;
    let r = dd_add(a, -dd_mul_f(b, q1));
    return quick_sum(q1, r.x / b.x);
}

// a point (hi.x, hi.y, lo.x, lo.y) minus the camera's centre as a double-f32 pair per axis: (x, y) rounded, (x, y)
// what the rounding left (drawing units)
fn rel2(p: vec4f, view: View) -> vec4f {
    let x1 = two_sum(p.x, -view.center.x);
    let y1 = two_sum(p.y, -view.center.y);
    let x2 = two_sum(p.z, -view.center.z);
    let y2 = two_sum(p.w, -view.center.w);
    let xs = two_sum(x1.x, x2.x);
    let ys = two_sum(y1.x, y2.x);
    let x = two_sum(xs.x, ((xs.y + x1.y) + x2.y) - view.rest.x);
    let y = two_sum(ys.x, ((ys.y + y1.y) + y2.y) - view.rest.y);
    return vec4f(x.x, y.x, x.y, y.y);
}

// drawing units relative to the centre -> pixels relative to the target's middle, y down (the drawing turns
// counter-clockwise by the rotation)
fn to_px(v: vec2f, view: View) -> vec2f {
    let c = view.turn.x;
    let s = view.turn.y;
    let r = vec2f(c * v.x - s * v.y, s * v.x + c * v.y) * view.rest.z;
    return vec2f(r.x, -r.y);
}

// half the stroke's width in pixels: drawing units scaled, never below half a pixel (thinner lines are drawn 1 px
// wide, as a hairline)
fn half_width(st: Style, view: View) -> f32 {
    var w = st.width;
    if (st.flags & 15u) == UNIT_DRAWING {
        w = w * view.rest.z;
    }
    return max(0.5 * w, 0.5);
}

// the join of a style: miter, round or bevel
fn join_of(st: Style) -> u32 {
    return (st.flags >> 8u) & 15u;
}

fn cap_of(st: Style) -> u32 {
    return (st.flags >> 4u) & 15u;
}

// the entries of a style's dash pattern (an odd pattern counted twice; 0: solid)
fn dash_count(st: Style) -> u32 {
    return (st.flags >> 12u) & 31u;
}

// drawing units per pixel of a dash pattern's unit
fn dash_scale(st: Style, view: View) -> f32 {
    return select(1.0, view.rest.w, ((st.flags >> 20u) & 15u) == UNIT_DRAWING);
}

// a miter fits the limit: 1 / cos(phi / 2) = sqrt(2 / (1 + c)) <= limit, c = cos(phi) of the turn (lib/gfx dash.rs's test)
fn miter_ok(c: f32, limit: f32) -> bool {
    let d1 = c + 1.0;
    return d1 > 1e-9 && 2.0 <= limit * limit * d1;
}

// the farthest any style's stroke reaches from its segment, in half widths (culling's bound: the turn's own angle
// is not looked at): a miter up to its limit, h tan(phi / 2) = h sqrt(limit^2 - 1), else a half width
fn reach_bound(st: Style) -> f32 {
    if join_of(st) == JOIN_MITER {
        return max(1.0, sqrt(max(st.miter * st.miter - 1.0, 0.0)));
    }
    return 1.0;
}

// how far an arc's piece bulges beyond its chord (px): the chord's half c, the radius r
fn sagitta(c: f32, r: f32) -> f32 {
    return c * c / (r + sqrt(max(r * r - c * c, 0.0)));
}

// an f32's fraction, exact
fn fr(x: f32) -> f32 {
    return x - floor(x);
}

fn acc(s: vec2f, x: f32) -> vec2f {
    return dd_add_f(s, fr(x));
}

// the dash pattern's place at the length l (three f32, exact) plus t (double-f32) along the entity: frac((l + t) k +
// p0) in periods, as double-f32 in [0, 1).  The products' exact pieces (two_prod) each lose their whole part (exact
// for an f32) and only the fractions are added: this holds for (l + t) k far beyond f32 (see the head)
fn periods(l: vec3f, t: vec2f, k: vec3f, p0: f32) -> vec2f {
    var s = vec2f(p0, 0.0);
    let a = two_prod(l.x, k.x);
    s = acc(s, a.x);
    s = acc(s, a.y);
    let b = two_prod(l.x, k.y);
    s = acc(s, b.x);
    s = acc(s, b.y);
    s = acc(s, l.x * k.z);
    let c = two_prod(l.y, k.x);
    s = acc(s, c.x);
    s = acc(s, c.y);
    s = acc(s, l.y * k.y);
    s = acc(s, l.z * k.x);
    let d = two_prod(t.x, k.x);
    s = acc(s, d.x);
    s = acc(s, d.y);
    let e = two_prod(t.x, k.y);
    s = acc(s, e.x);
    s = acc(s, e.y);
    s = acc(s, t.x * k.z);
    let f = two_prod(t.y, k.x);
    s = acc(s, f.x);
    s = acc(s, f.y);
    s = acc(s, t.y * k.y);
    var r = quick_sum(s.x - floor(s.x), s.y);
    if r.x < 0.0 {
        r = dd_add_f(r, 1.0);
    }
    if r.x >= 1.0 {
        r = dd_add_f(r, -1.0);
    }
    return r;
}

// beyond this many pixels from the target's middle at both ends, a segment's line is placed by far_foot
const FAR: f32 = 131072.0;

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

// a fill's edge from r0 to r1 (rel2's pairs) in pixels of the target (from its top left corner), clipped to the target
// and 8 px around it; both ends beyond FAR: its line placed by far_foot (as strokes); clipped away: far outside.  The count
// pass writes it per edge record (cull.wgsl), and every record of the fill reads those bits
fn edge_px(r0: vec4f, r1: vec4f) -> vec4f {
    let half = 0.5 * view.turn.zw;
    let p0 = to_px(r0.xy, view);
    let p1 = to_px(r1.xy, view);
    let lo = -half - vec2f(8.0);
    let hi = half + vec2f(8.0);
    // (wholly beside the target on one side, with what f32 may round far points to: far outside at once)
    let mg = 1.0 + 4e-6 * (length(p0) + length(p1));
    if max(p0.x, p1.x) < lo.x - mg || min(p0.x, p1.x) > hi.x + mg || max(p0.y, p1.y) < lo.y - mg || min(p0.y, p1.y) > hi.y + mg {
        return vec4f(-1e6);
    }
    let dv = p1 - p0;
    let len0 = length(dv);
    var d = vec2f(1.0, 0.0);
    if len0 > 0.0 {
        d = dv / len0;
    }
    var base = p0;
    var s0 = 0.0;
    var s1 = len0;
    if max(abs(p1.x), abs(p1.y)) < max(abs(p0.x), abs(p0.y)) {
        base = p1;
        s0 = -len0;
        s1 = 0.0;
    }
    let far = min(max(abs(p0.x), abs(p0.y)), max(abs(p1.x), abs(p1.y))) > FAR;
    if far {
        if !(length(r1.xy - r0.xy) > 0.0) {
            return vec4f(-1e6);
        }
        base = far_foot(r0, r1);
        s0 = dot(p0 - base, d);
        s1 = dot(p1 - base, d);
    }
    var sa = s0;
    var sb = s1;
    for (var k = 0u; k < 2u; k++) {
        let b = base[k];
        let dk = d[k];
        if abs(dk) < 1e-12 {
            if b < lo[k] || b > hi[k] {
                return vec4f(-1e6);
            }
        } else {
            let ta = (lo[k] - b) / dk;
            let tb = (hi[k] - b) / dk;
            sa = max(sa, min(ta, tb));
            sb = min(sb, max(ta, tb));
        }
    }
    if sa > sb {
        return vec4f(-1e6);
    }
    var q0 = p0;
    var q1 = p1;
    if sa > s0 || far {
        q0 = base + sa * d;
    }
    if sb < s1 || far {
        q1 = base + sb * d;
    }
    return vec4f(q0 + half, q1 + half);
}
