
// fill: fills and hatches (decision 26; docs/RENDER2D.md "Flächen", "Schraffuren"), the third pipeline (vs_fill,
// fs_fill) for the layers that hold a fill; the strokes of such a layer it draws as fs_full does (full_body).
//
// A fill's records: its rings' edges (K_EDGE, each ring a closed polyline of them, the inside on the right in pixels:
// scene.rs orients the rings) and its triangles (K_TRI, their corners the starts of edge records).  The count pass
// writes every edge's segment in pixels to `epx` (cull.wgsl: edge_px), so that whoever needs an edge reads the same
// bits.  Each record carries the edges near it (scene.rs, fill.rs near: within a pixel at the coarse end of the zoom
// band): a triangle up to 20, an edge up to 4 besides its two neighbours.  The rasterizer only brings candidates
// (conservative quads); the shader decides every pixel, so that each one blends once:
//   - a pixel within MU of an edge, and a pixel outside the fill whose square still touches it (within FILL_R), belongs to
//     the nearest edge (among an edge's own, its neighbours and its list; ties to the incoming neighbour, else to the
//     record placed first)
//   - every other pixel inside belongs to the triangle that holds its centre: three lines per triangle, each computed by
//     tri_line from the same corners in the same order by both triangles at a diagonal, a centre on it to the one on its
//     positive side
// The owner computes the pixel's share inside the fill exactly, by Green's theorem over the edges it knows: W(ref) for
// a point ref of the pixel whose side is sure, plus per edge -∫ (x - x_left) dy along its part inside the pixel and the
// crossings of the path from ref to the pixel's right side and along that side (fill_contrib).  Exact where the owner
// knows every edge through the pixel: everywhere but where more edges than its list holds come that near (details far
// below a pixel); a triangle's pixel farther than FILL_R from its edges is simply covered.
//
// Hatches (the style's place in the hatch table + 1 in misc.w): per family the pixel's distance from the nearest line
// and its place along it, from what scene.rs computed at the camera in f64, so that the numbers here stay small (no
// drift at 1e7 and 1e-6 units per pixel): the line's strip by cov, its dashes along it by cov, the families together
// as 1 - prod(1 - c); below LINE_FLAT px between lines (DASH_FLAT for a dash period) a flat tone of the lines' share,
// exact from LINE_EXACT (DASH_EXACT), mixed between.  The fill's coverage times the hatch's.

// a pixel this close (px) to an edge belongs to the edges; beyond FILL_R of all, a pixel's square touches none
const MU: f32 = 0.015625;
const FILL_R: f32 = 0.7072;
// hatch lines closer than LINE_FLAT px: a flat tone; from LINE_EXACT on exactly; mixed between
const LINE_FLAT: f32 = 2.0;
const LINE_EXACT: f32 = 4.0;
// misc.x of VsFill: a fill's record, a triangle
const M_FILL: u32 = 262144u;
const M_TRI: u32 = 524288u;

// a hatched style's entry (scene.rs hatch_words): x the families, y the lines' half width (px); per family f from
// v[1 + 6 f]: (normal in px, the camera centre's distance from the nearest line px, the spacing px), (direction in px,
// the place along that line at the centre's foot, the dash period; drawing units, period 0: solid), (the shift between
// lines, the share on, the entries), then up to 12 ends of the entries from the period's start (drawing units)
struct Hatch {
    v: array<vec4f, 32>,
}

@group(0) @binding(5) var<storage, read> hatches: array<Hatch>;
// per record of the pool: a fill edge's segment in pixels of the target (cull.wgsl count writes it)
@group(0) @binding(6) var<storage, read> epx: array<vec4f>;

struct VsFill {
    @builtin(position) pos: vec4f,
    // strokes: as VsOut
    @location(0) @interpolate(flat) seg: vec4f,
    @location(1) @interpolate(flat) nb: vec4f,
    @location(2) @interpolate(flat) par: vec4f,
    // flags (M_*), colour, style, hatch + 1
    @location(3) @interpolate(flat) misc: vec4u,
    @location(4) @interpolate(flat) dash: vec4f,
    // a triangle's lines: normal, offset, its side (+1, -1; 0: none)
    @location(5) @interpolate(flat) l0: vec4f,
    @location(6) @interpolate(flat) l1: vec4f,
    @location(7) @interpolate(flat) l2: vec4f,
    // the record, an edge's previous and next one
    @location(8) @interpolate(flat) ix: vec4u,
}

fn fill_out() -> VsFill {
    var o: VsFill;
    o.pos = vec4f(2.0, 2.0, 0.5, 1.0);
    o.seg = vec4f(0.0);
    o.nb = vec4f(0.0);
    o.par = vec4f(0.0);
    o.misc = vec4u(0u);
    o.dash = vec4f(0.0);
    o.l0 = vec4f(0.0);
    o.l1 = vec4f(0.0);
    o.l2 = vec4f(0.0);
    o.ix = vec4u(0u);
    return o;
}

// word j of a record (0 to 15)
fn rec_word(r: Record, j: u32) -> u32 {
    let q = j >> 2u;
    var v = r.w;
    if q == 0u {
        v = bitcast<vec4u>(r.a);
    } else if q == 1u {
        v = bitcast<vec4u>(r.b);
    } else if q == 2u {
        v = r.v;
    }
    return v[j & 3u];
}

// entry k of a list of u16 from word j0 on
fn rec_u16(r: Record, j0: u32, k: u32) -> u32 {
    return (rec_word(r, j0 + (k >> 1u)) >> (16u * (k & 1u))) & 0xffffu;
}

// the line of a triangle's edge from r0 to r1 (rel2's pairs) in pixels of the target: its unit normal and offset
// (dot(q, n) = offset on it), precise near the camera (far_foot's foot where both ends lie far); the triangles at a
// diagonal call it with the same corners in the same order
fn tri_line(r0: vec4f, r1: vec4f) -> vec3f {
    let half = 0.5 * view.turn.zw;
    let p0 = to_px(r0.xy, view);
    let p1 = to_px(r1.xy, view);
    let dd = vec2f((r1.x - r0.x) + (r1.z - r0.z), (r1.y - r0.y) + (r1.w - r0.w));
    let dp = to_px(dd, view);
    let n = unit_or_zero(vec2f(-dp.y, dp.x));
    var foot = p0;
    if min(max(abs(p0.x), abs(p0.y)), max(abs(p1.x), abs(p1.y))) > FAR && length(r1.xy - r0.xy) > 0.0 {
        foot = far_foot(r0, r1);
    }
    return vec3f(n, dot(foot + half, n));
}

// the line with the side the triangle's third corner w (px of the target) lies on
fn tri_side(l: vec3f, w: vec2f) -> vec4f {
    let g = dot(w, l.xy) - l.z;
    return vec4f(l, select(select(0.0, -1.0, g < 0.0), 1.0, g > 0.0));
}

// the centre q on the triangle's side of a line (on the line: the triangle on its positive side)
fn in_line(l: vec4f, q: vec2f) -> bool {
    let g = dot(q, l.xy) - l.z;
    return l.w * g > 0.0 || (g == 0.0 && l.w > 0.0);
}

// a fill's record: an edge's quad (its segment and FILL_R + 1 px around), a triangle's (its box along its longest side
// and a pixel, or near a far corner the box of its corners within the target), its lines
fn fill_vertex(vi: u32, idx: u32, r: Record, st: Style) -> VsFill {
    var o = fill_out();
    let half = 0.5 * view.turn.zw;
    if kind_of_record(r) == K_EDGE {
        let ip = select(idx - 1u, idx + r.w.y, (r.w.w & F_WRAP_PREV) != 0u);
        let inx = select(idx + 1u, idx - r.w.y, (r.w.w & F_WRAP_NEXT) != 0u);
        let e1 = epx[idx];
        if e1.x == -1e6 && e1.z == -1e6 {
            return o;
        }
        let a = e1.xy - half;
        let dv = e1.zw - e1.xy;
        let l = length(dv);
        var d = vec2f(1.0, 0.0);
        if l > 0.0 {
            d = dv / l;
        }
        let n = vec2f(-d.y, d.x);
        let w = FILL_R + 1.0;
        let p = a + select(-w, l + w, (vi & 1u) != 0u) * d + select(-w, w, (vi & 2u) != 0u) * n;
        o.pos = vec4f(p.x / half.x, -p.y / half.y, 0.5, 1.0);
        o.misc = vec4u(M_FILL, st.color, r.w.z - 1u, st.hatch);
        o.ix = vec4u(idx, ip, inx, 0u);
        return o;
    }
    // a triangle: its corners are the starts of edge records (a.yzw from the fill's first record, a.x back)
    let base = idx - bitcast<u32>(r.a.x);
    let ia = base + bitcast<u32>(r.a.y);
    let ib = base + bitcast<u32>(r.a.z);
    let ic = base + bitcast<u32>(r.a.w);
    let ra = rel2(pool[ia].a, view);
    let rb = rel2(pool[ib].a, view);
    let rc = rel2(pool[ic].a, view);
    let qa = to_px(ra.xy, view);
    let qb = to_px(rb.xy, view);
    let qc = to_px(rc.xy, view);
    // the lines, each from its corners in the order of their records
    var lab = tri_line(ra, rb);
    if ib < ia {
        lab = tri_line(rb, ra);
    }
    var lbc = tri_line(rb, rc);
    if ic < ib {
        lbc = tri_line(rc, rb);
    }
    var lca = tri_line(rc, ra);
    if ia < ic {
        lca = tri_line(ra, rc);
    }
    o.l0 = tri_side(lab, qc + half);
    o.l1 = tri_side(lbc, qa + half);
    o.l2 = tri_side(lca, qb + half);
    o.misc = vec4u(M_FILL | M_TRI, st.color, r.w.z - 1u, st.hatch);
    o.ix = vec4u(idx, 0u, 0u, 0u);
    var p: vec2f;
    if max(max(max(abs(qa.x), abs(qa.y)), max(abs(qb.x), abs(qb.y))), max(abs(qc.x), abs(qc.y))) < 4096.0 {
        // the box along its longest side, a pixel around
        var u = qa;
        var v = qb;
        var w = qc;
        if dot(qc - qb, qc - qb) > dot(v - u, v - u) {
            u = qb;
            v = qc;
            w = qa;
        }
        if dot(qa - qc, qa - qc) > dot(v - u, v - u) {
            u = qc;
            v = qa;
            w = qb;
        }
        let e = v - u;
        let le = length(e);
        if !(le > 0.0) {
            return o;
        }
        let d = e / le;
        var n = vec2f(-d.y, d.x);
        var ht = dot(w - u, n);
        if ht < 0.0 {
            n = -n;
            ht = -ht;
        }
        let tw = dot(w - u, d);
        p = u + select(min(0.0, tw) - 1.0, max(le, tw) + 1.0, (vi & 1u) != 0u) * d + select(-1.0, ht + 1.0, (vi & 2u) != 0u) * n;
    } else {
        // (a corner far away: its place in f32 is coarse; the box of the corners within the target and 2 px)
        let lo = max(min(min(qa, qb), qc) - vec2f(2.0), -half - vec2f(2.0));
        let hi = min(max(max(qa, qb), qc) + vec2f(2.0), half + vec2f(2.0));
        if any(lo > hi) {
            return o;
        }
        p = vec2f(select(lo.x, hi.x, (vi & 1u) != 0u), select(lo.y, hi.y, (vi & 2u) != 0u));
    }
    o.pos = vec4f(p.x / half.x, -p.y / half.y, 0.5, 1.0);
    return o;
}

@vertex
fn vs_fill(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> VsFill {
    let l = lists[pc.layer + (ii >> 16u)];
    let j = (ii & 0xffffu) * BATCH + (vi >> 2u);
    if j < l.y {
        let idx = visible[l.x + j];
        let r = pool[idx];
        let k = kind_of_record(r);
        if k == K_EDGE || k == K_TRI {
            return fill_vertex(vi, idx, r, styles[r.w.z - 1u]);
        }
    }
    let s = vertex(vi, ii, true);
    var o = fill_out();
    o.pos = s.pos;
    o.seg = s.seg;
    o.nb = s.nb;
    o.par = s.par;
    o.misc = s.misc;
    o.dash = s.dash;
    return o;
}

// the record of a fill's pixel's k-th edge: a triangle's list (words 4 to 13, u16 from the fill's first record); an
// edge's previous one, itself, its next one, then its list (words 11, 12)
fn fill_cand(tri: bool, rec: Record, ix: vec4u, base: u32, k: u32) -> u32 {
    if tri {
        return base + rec_u16(rec, 4u, k);
    }
    if k == 0u {
        return ix.y;
    }
    if k == 1u {
        return ix.x;
    }
    if k == 2u {
        return ix.z;
    }
    return base + rec_u16(rec, 11u, k - 3u);
}

// a + t (p, q) narrowed to p t <= q (Liang-Barsky)
fn lb(p: f32, q: f32, t: vec2f) -> vec2f {
    if p == 0.0 {
        if q < 0.0 {
            return vec2f(1.0, 0.0);
        }
        return t;
    }
    let r = q / p;
    if p < 0.0 {
        return vec2f(max(t.x, r), t.y);
    }
    return vec2f(t.x, min(t.y, r));
}

// an edge's part of the pixel's share (see the head; everything relative to the pixel's centre, the pixel [-0.5, 0.5]^2,
// ref the point whose side is known): -∫ (x + 0.5) dy along the edge inside the pixel, the crossing of the path from ref
// to the right side (a downward edge enters the fill: +1 for all of the side), the crossing of the right side at y
// (from ref.y outwards the side's inside changes by the edge's direction)
fn fill_contrib(r: vec2f, a: vec2f, b: vec2f) -> f32 {
    let d = b - a;
    var t = vec2f(0.0, 1.0);
    t = lb(-d.x, a.x + 0.5, t);
    t = lb(d.x, 0.5 - a.x, t);
    t = lb(-d.y, a.y + 0.5, t);
    t = lb(d.y, 0.5 - a.y, t);
    var s = 0.0;
    if t.y > t.x {
        let p = a + t.x * d;
        let e = a + t.y * d;
        s = -(0.5 * (p.x + e.x) + 0.5) * (e.y - p.y);
    }
    if (a.y > r.y) != (b.y > r.y) {
        let xi = a.x + (r.y - a.y) * d.x / d.y;
        if xi > r.x && xi <= 0.5 {
            s += sign(d.y);
        }
    }
    if (a.x > 0.5) != (b.x > 0.5) {
        let ye = a.y + (0.5 - a.x) * d.y / d.x;
        if ye > r.y && ye <= 0.5 {
            s -= sign(d.x) * (0.5 - ye);
        } else if ye >= -0.5 && ye < r.y {
            s += sign(d.x) * (ye + 0.5);
        }
    }
    return s;
}

// a hatch family's line j at distance x (px) from the pixel's centre: its strip, times its dashes along it
fn hatch_line(x: f32, j: f32, qc: vec2f, n: vec2f, hw: f32, b: vec4f, c: vec4f, slot: u32, f: u32) -> f32 {
    let across = max(cov(x - hw, n) + cov(-x - hw, -n) - 1.0, 0.0);
    let per = b.w;
    if across <= 0.0 || per <= 0.0 {
        return across;
    }
    let sc = view.rest.w;
    let t = b.xy;
    // the pixel's place along the line, drawing units, within [0, per)
    let x0 = b.z + (dot(qc, t) * sc - j * c.x);
    let xr = x0 - floor(x0 / per) * per;
    let tfa = clamp((DASH_EXACT - per / sc) / (DASH_EXACT - DASH_FLAT), 0.0, 1.0);
    var al = 0.0;
    if tfa < 1.0 {
        let m = u32(c.z);
        var a0 = 0.0;
        for (var k = 0u; k < m; k += 2u) {
            let a1 = hatches[slot].v[4u + 6u * f + (k >> 2u)][k & 3u];
            for (var cp = -1.0; cp < 1.5; cp += 1.0) {
                var ua = (a0 + cp * per - xr) / sc;
                var ub = (a1 + cp * per - xr) / sc;
                if ub - ua < 2.0 * hw {
                    // (a dot: as long as the line is wide)
                    let mid = 0.5 * (ua + ub);
                    ua = mid - hw;
                    ub = mid + hw;
                }
                al += max(cov(-ub, t) + cov(ua, -t) - 1.0, 0.0);
            }
            if k + 1u < m {
                a0 = hatches[slot].v[4u + 6u * f + ((k + 1u) >> 2u)][(k + 1u) & 3u];
            }
        }
    }
    return across * mix(min(al, 1.0), c.y, tfa);
}

// a hatch's coverage of the pixel at q (see the head)
fn hatch_cov(slot: u32, q: vec2f) -> f32 {
    let qc = q - 0.5 * view.turn.zw;
    let hd = hatches[slot].v[0];
    let nf = u32(hd.x);
    let hw = hd.y;
    var keep = 1.0;
    for (var f = 0u; f < nf; f++) {
        let a = hatches[slot].v[1u + 6u * f];
        let b = hatches[slot].v[2u + 6u * f];
        let c = hatches[slot].v[3u + 6u * f];
        let sp = a.w;
        let dl = a.z + dot(qc, a.xy);
        let tf = clamp((LINE_EXACT - sp) / (LINE_EXACT - LINE_FLAT), 0.0, 1.0);
        let tone = min(1.0, 2.0 * hw / sp) * c.y;
        var ex = 0.0;
        if tf < 1.0 {
            let j = round(dl / sp);
            let x = dl - j * sp;
            let j2 = j + select(-1.0, 1.0, x > 0.0);
            ex = min(1.0, hatch_line(x, j, qc, a.xy, hw, b, c, slot, f) + hatch_line(dl - j2 * sp, j2, qc, a.xy, hw, b, c, slot, f));
        }
        keep *= 1.0 - mix(ex, tone, tf);
    }
    return 1.0 - keep;
}

// a fill's pixel (see the head)
fn fill_body(i: VsFill) -> vec4f {
    let q = i.pos.xy;
    let tri = (i.misc.x & M_TRI) != 0u;
    let col = unpack4x8unorm(i.misc.y);
    let idx = i.ix.x;
    let rec = pool[idx];
    var base = 0u;
    var n = 0u;
    if tri {
        if !(in_line(i.l0, q) && in_line(i.l1, q) && in_line(i.l2, q)) {
            discard;
            return vec4f(0.0);
        }
        base = idx - bitcast<u32>(rec.a.x);
        n = rec.w.w >> 16u;
    } else {
        base = idx - rec.v.z;
        n = 3u + (rec.w.w >> 16u);
    }
    // the squared distances from the edges, at one place for every owner (both sides of each comparison compute them alike)
    var dm = 3.4e38;
    var dp = 0.0;
    var dn = 0.0;
    var d1 = 0.0;
    var beaten = false;
    for (var k = 0u; k < n; k++) {
        let e = fill_cand(tri, rec, i.ix, base, k);
        let s = epx[e];
        let d = dist_seg(q - s.xy, s.zw - s.xy);
        dm = min(dm, d);
        if k == 0u {
            dp = d;
        } else if k == 1u {
            d1 = d;
        } else if k == 2u {
            dn = d;
        } else if d < d1 || (d == d1 && e < idx) {
            // (an edge of another part of the outline, nearer: its pixel)
            beaten = true;
        }
    }
    var w = 1.0;
    var r = vec2f(0.0);
    var m = n;
    if tri {
        if dm <= MU * MU {
            discard;
            return vec4f(0.0);
        }
        if dm > FILL_R * FILL_R {
            // (no edge touches the pixel)
            m = 0u;
        }
    } else {
        // the nearest edge (ties to the incoming neighbour), within reach
        if !(d1 < dp && d1 <= dn) || beaten || d1 > FILL_R * FILL_R {
            discard;
            return vec4f(0.0);
        }
        // outside: by the side of the edge, at its end by the sum of both edges' outer normals (the inside on the right)
        let s1 = epx[idx];
        let a = s1.xy;
        let ab = s1.zw - a;
        let ll = dot(ab, ab);
        var t = 0.0;
        if ll > 0.0 {
            t = dot(q - a, ab) / ll;
        }
        var nrm = vec2f(-ab.y, ab.x);
        var o = q - a;
        if t >= 1.0 {
            let s2 = epx[i.ix.z];
            let bn = s2.zw - s2.xy;
            nrm = unit_or_zero(nrm) + unit_or_zero(vec2f(-bn.y, bn.x));
            o = q - s1.zw;
        }
        let outside = dot(o, nrm) > 0.0;
        if !(outside || d1 <= MU * MU) {
            discard;
            return vec4f(0.0);
        }
        w = select(1.0, 0.0, outside);
        if d1 < 1e-6 {
            // (on the edge: a point a little outside it is the one whose side is sure)
            r = 2e-3 * unit_or_zero(nrm);
            w = 0.0;
        }
    }
    var c = w;
    for (var k = 0u; k < m; k++) {
        let s = epx[fill_cand(tri, rec, i.ix, base, k)];
        c += fill_contrib(r, s.xy - q, s.zw - q);
    }
    c = clamp(c, 0.0, 1.0);
    if i.misc.w != 0u {
        c *= hatch_cov(i.misc.w - 1u, q);
    }
    if !(c > 0.0) {
        discard;
        return vec4f(0.0);
    }
    let al = col.w * c;
    return vec4f(vec3f(col.z, col.y, col.x) * al, al);
}

@fragment
fn fs_fill(i: VsFill) -> @location(0) vec4f {
    if (i.misc.x & M_FILL) != 0u {
        return fill_body(i);
    }
    var o: VsOut;
    o.pos = i.pos;
    o.seg = i.seg;
    o.nb = i.nb;
    o.par = i.par;
    o.misc = i.misc;
    o.dash = i.dash;
    return full_body(o);
}
