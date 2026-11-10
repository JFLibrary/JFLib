// common -- what cull.wgsl and stroke.wgsl share (lib.rs puts it in front of both): the view, records, styles, the
// coordinates relative to the camera and how far a segment's stroke reaches (docs/RENDER2D.md)
//
// Coordinates (decision 28): a point is f64 on the CPU and a pair of f32 per axis here, hi = f32(x) and lo = f32(x -
// hi).  The camera's centre comes the same way, and what f32 cannot hold of it (`rest`, at most 2^-26 of a unit at
// 1e7) is taken off afterwards.  rel() subtracts the camera first: hi - hi and lo - lo each through an exact two-sum,
// so a point near the camera comes out to a few 1e-15 units; a point far from it as a double-f32 pair (rel2), whose
// rounded part has f32's 24 bits of its distance.  Pixels are then y down from the target's top left corner.

struct View {
    // the camera's centre: hi.x, hi.y, lo.x, lo.y (drawing units)
    center: vec4f,
    // what hi + lo leaves of it (x, y), 1 / scale (pixels per unit), scale (units per pixel)
    rest: vec4f,
    // cos and sin of the rotation, the target's width and height (px)
    turn: vec4f,
    // x: 1 when culling is off; the rest unused
    misc: vec4f,
}

// a record: one segment of an entity, 64 bytes
struct Record {
    // the segment's start and end: hi.x, hi.y, lo.x, lo.y
    a: vec4f,
    b: vec4f,
    // the previous point minus the start, the next point minus the end (drawing units); (0, 0): none (an end)
    near: vec4f,
    // style + 1 (0: a free record); the end minus the start as two f32 bits (exactly the negated previous vector of the
    // next record and the next vector of the one before: a join's bisector comes out the same in both); where the
    // polyline ends (1: its previous point is its first, a cap there; 2: its next point is its last) | kind << 8
    // (0: a straight segment)
    info: vec4u,
}

// a style, 32 bytes (lib.rs Style::words)
struct Style {
    // 0xAARRGGBB, sRGB codes, straight alpha
    color: u32,
    width: f32,
    // unit | cap << 4 | join << 8
    flags: u32,
    // miter limit: the longest miter over the width
    miter: f32,
    r0: u32,
    r1: u32,
    r2: u32,
    r3: u32,
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

// a + b exactly as s + e (Knuth's two-sum; round to nearest, nothing reassociated)
fn two_sum(a: f32, b: f32) -> vec2f {
    let s = a + b;
    let bb = s - a;
    let e = (a - (s - bb)) + (b - bb);
    return vec2f(s, e);
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
