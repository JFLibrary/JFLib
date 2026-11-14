
// text: glyphs from the atlas of distance fields and small text's bars (G6c, decision 27; docs/RENDER2D.md "Text").  lib.rs
// puts it behind stroke.wgsl and fill.wgsl (TEXT_WGSL): vs_text and fs_text draw layers that hold nothing but text, vs_text_all
// and fs_text_all layers that hold text among anything else (fs_fill's work and text's: the painter's order within a layer
// stays, one pipeline draws the layer).  Their varyings are VsFill's; a text record packs its own into them (text_vertex).
//
// A glyph's quad covers its quad box (its ink, and where a neighbour's ink reaches into its cell) through the em -> pixel map
// and a pixel more.  The fragment shader maps the pixel's centre back to em units (the inverse of the map: flat, from the
// vertex shader) and reads the glyph's tile in the atlas: the four texels around the point (textureLoad, the bilinear weights
// in f32: no sampler and its 8-bit weights), the median of red, green and blue and that channel's gradient, which the map's
// Jacobian turns into pixels: the distance of the edge from the centre in pixels and its normal, so that width factors,
// oblique angles and uneven inserts stay sharp, without fwidth in a branch.  The coverage is the box filter of that edge
// (cov, exact for a straight one).
//
// Every pixel blends once per text: a glyph owns the pixels whose centre lies in its cell along the baseline (from its pen
// to the next glyph's; both compute the bound from the same bits) and adds its neighbours' coverage where their ink comes
// near.  Marks (accents over a base, no advance) draw their own ink alone.
//
// Size classes: S (32 texels per em) for small and medium text, L (64) from T_SL pixels per em on (the map's larger stretch at
// the decided scale); a glyph without a tile of the wanted class takes the other.

// misc.x of a text record's VsFill: text, a bar, class L, a previous and a next neighbour
const M_TEXT: u32 = 1048576u;
const M_BAR: u32 = 2097152u;
const M_CLS_L: u32 = 4194304u;
const M_PREVG: u32 = 8388608u;
const M_NEXTG: u32 = 16777216u;
const CLS_S_EM: f32 = 32.0;
const CLS_S_RANGE: f32 = 8.0;
const CLS_L_EM: f32 = 64.0;
const CLS_L_RANGE: f32 = 4.0;
// from this many pixels per em (the map's larger stretch) class L
const T_SL: f32 = 40.0;
const NO_TILE: u32 = 0xffffffffu;

// a glyph of the table (atlas.rs GlyphRow::words): its ink box from its pen (em); per class its tile: x | y << 10 | page << 20
// (NO_TILE: none), w | h << 16, its texel origin (x0, y0 as f32)
struct Glyph {
    ink: vec4f,
    s: vec4u,
    l: vec4u,
    r: vec4u,
}

@group(0) @binding(9) var atlas: texture_2d_array<f32>;
@group(0) @binding(10) var<storage, read> gtab: array<Glyph>;

// the class a glyph is drawn from: L where wanted and there, S where there, else L (true: L)
fn class_of(g: Glyph, large: bool) -> bool {
    return select(g.s.x == NO_TILE, g.l.x != NO_TILE, large);
}

// a text record's quad (it: the record, an insert's through its space; layer_t: its layer's transparency)
fn text_vertex(vi: u32, it: Item, r: Record, st: Style, layer_t: u32) -> VsFill {
    var o = fill_out();
    let vw = it.vw;
    let a = text_map(r, vw);
    let bar = kind_of_record(r) == K_BAR;
    if text_glyphs(a) == bar {
        return o;
    }
    let det = a[0].x * a[1].y - a[1].x * a[0].y;
    if !(abs(det) > 1e-30) {
        return o;
    }
    let half = 0.5 * vw.turn.zw;
    // the text's origin in pixels from the target's middle
    let org = to_px(rel2(r.a, vw).xy, vw);
    let b = text_box(r);
    // the corners a pixel out along both axes of the map: m / |sin| along each moves its sides by a pixel
    let u = normalize(a[0]);
    let v = normalize(a[1]);
    let mg = 1.0 / max(abs(u.x * v.y - u.y * v.x), 0.1);
    let hx = (vi & 1u) != 0u;
    let hy = (vi & 2u) != 0u;
    let e = vec2f(select(b.x, b.z, hx), select(b.y, b.w, hy));
    let p = org + a * e + mg * (select(-1.0, 1.0, hx) * u + select(-1.0, 1.0, hy) * v);
    o.pos = vec4f(p.x / half.x, -p.y / half.y, 0.5, 1.0);
    // pixels -> em: the inverse map
    let inv = mat2x2f(vec2f(a[1].y, -a[0].y), vec2f(-a[1].x, a[0].x)) * (1.0 / det);
    o.seg = vec4f(org + half, inv[0]);
    let color = ink(st.color, st.r1.x, layer_t);
    if bar {
        o.nb = vec4f(inv[1], 0.0, 0.0);
        o.par = vec4f(bitcast<f32>(r.w.x), 0.0, 0.0, 0.0);
        o.l1 = vec4f(bitcast<f32>(r.v.x), bitcast<f32>(r.v.y), bitcast<f32>(r.v.z), bitcast<f32>(r.v.w));
        o.misc = vec4u(M_TEXT | M_BAR, color, 0u, 0u);
        return o;
    }
    let idx = it.idx;
    let g = gtab[r.v.z];
    // the class by the map's larger stretch at the decided scale
    let s2 = dot(a[0], a[0]) + dot(a[1], a[1]);
    let big = sqrt(0.5 * (s2 + sqrt(max(s2 * s2 - 4.0 * det * det, 0.0)))) * view.misc.z;
    let cls = class_of(g, big >= T_SL);
    let t = select(g.s, g.l, cls);
    if t.x == NO_TILE {
        return fill_out();
    }
    // its cell: from its previous neighbour's right end to its own (a mark, or a line's ends: open)
    let pb = (r.w.w >> 16u) & 255u;
    let nx = (r.w.w >> 24u) & 255u;
    let mark = (r.w.w & G_MARK) != 0u;
    var cl = -3.0e38;
    var cr = 3.0e38;
    var flags = M_TEXT | select(0u, M_CLS_L, cls);
    if !mark {
        if pb != 0u {
            cl = bitcast<f32>(pool[idx - pb].v.w);
            flags |= M_PREVG;
        }
        if nx != 0u {
            cr = bitcast<f32>(r.v.w);
            flags |= M_NEXTG;
        }
    }
    o.nb = vec4f(inv[1], bitcast<vec2f>(r.v.xy));
    o.par = vec4f(cl, cr, bitcast<f32>(t.z), bitcast<f32>(t.w));
    o.misc = vec4u(flags, color, t.x, t.y);
    o.ix = vec4u(idx, idx - pb, idx + nx, 0u);
    return o;
}

// the box filter of a straight edge s from the centre (s > 0: outside), m its normal (stroke.wgsl cov)
fn tcov(s: f32, m: vec2f) -> f32 {
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

// the coverage of a glyph at el (em from its pen) from its tile (loc, wh, its texel origin x0) of a class (em: texels per em,
// range: texels across), inv the pixel -> em map
fn glyph_cov(el: vec2f, loc: u32, wh: u32, x0: vec2f, inv: mat2x2f, em: f32, range: f32) -> f32 {
    let size = vec2f(f32(wh & 0xffffu), f32(wh >> 16u));
    // texel coordinates in the tile (texel i's centre at i + 0.5); beyond its margin no ink
    let t = el * em - x0;
    if any(t < vec2f(-1.0)) || any(t > size + vec2f(1.0)) {
        return 0.0;
    }
    let tx = t - vec2f(0.5);
    let f0 = floor(tx);
    let f = tx - f0;
    let lim = vec2i(size) - vec2i(1);
    let i0 = clamp(vec2i(f0), vec2i(0), lim);
    let i1 = clamp(vec2i(f0) + vec2i(1), vec2i(0), lim);
    let base = vec2i(i32(loc & 1023u), i32((loc >> 10u) & 1023u));
    let page = i32(loc >> 20u);
    let c00 = textureLoad(atlas, base + i0, page, 0);
    let c10 = textureLoad(atlas, base + vec2i(i1.x, i0.y), page, 0);
    let c01 = textureLoad(atlas, base + vec2i(i0.x, i1.y), page, 0);
    let c11 = textureLoad(atlas, base + i1, page, 0);
    let lo = mix(c00, c10, f.x);
    let hi = mix(c01, c11, f.x);
    let v = mix(lo, hi, f.y);
    let gx = mix(c10 - c00, c11 - c01, f.y);
    let gy = hi - lo;
    // the median channel and its gradient
    var d = v.y;
    var g = vec2f(gx.y, gy.y);
    if (v.x <= v.y && v.y <= v.z) || (v.z <= v.y && v.y <= v.x) {
        d = v.y;
    } else if (v.y <= v.x && v.x <= v.z) || (v.z <= v.x && v.x <= v.y) {
        d = v.x;
        g = vec2f(gx.x, gy.x);
    } else {
        d = v.z;
        g = vec2f(gx.z, gy.z);
    }
    let sd = (d - 0.5) * range;
    // texels per pixel: em * inv; the gradient in pixels is its transpose times the texels' one
    let gq = em * range * vec2f(dot(inv[0], g), dot(inv[1], g));
    let l = length(gq);
    if !(l > 1e-6) {
        return select(0.0, 1.0, sd > 0.0);
    }
    return tcov(-sd / l, gq / l);
}

// a neighbour's coverage at e (em from the text's origin): record k, the class the glyph takes; none where the pixel lies
// beyond its ink box and a margin mg (em)
fn neighbour_cov(e: vec2f, k: u32, large: bool, inv: mat2x2f, mg: f32) -> f32 {
    let r = pool[k];
    let g = gtab[r.v.z];
    let el = e - bitcast<vec2f>(r.v.xy);
    if el.x < g.ink.x - mg || el.x > g.ink.z + mg || el.y < g.ink.y - mg || el.y > g.ink.w + mg {
        return 0.0;
    }
    let cls = class_of(g, large);
    let t = select(g.s, g.l, cls);
    if t.x == NO_TILE {
        return 0.0;
    }
    return glyph_cov(el, t.x, t.y, bitcast<vec2f>(t.zw), inv, select(CLS_S_EM, CLS_L_EM, cls), select(CLS_S_RANGE, CLS_L_RANGE, cls));
}

// a bar's coverage: the strips across and along its box (x0, x1, y0, y1 em), their product
fn bar_cov(e: vec2f, inv: mat2x2f, b: vec4f) -> f32 {
    let gx = vec2f(inv[0].x, inv[1].x);
    let gy = vec2f(inv[0].y, inv[1].y);
    let lx = length(gx);
    let ly = length(gy);
    let nx = gx / lx;
    let ny = gy / ly;
    let cx = max(tcov((b.x - e.x) / lx, nx) + tcov((e.x - b.y) / lx, nx) - 1.0, 0.0);
    let cy = max(tcov((b.z - e.y) / ly, ny) + tcov((e.y - b.w) / ly, ny) - 1.0, 0.0);
    return cx * cy;
}

// a text record's pixel
fn text_body(i: VsFill) -> vec4f {
    let inv = mat2x2f(i.seg.zw, i.nb.xy);
    let e = inv * (i.pos.xy - i.seg.xy);
    var c = 0.0;
    if (i.misc.x & M_BAR) != 0u {
        c = bar_cov(e, inv, i.l1) * i.par.x;
    } else {
        // (a pixel of another glyph's cell is that one's)
        if e.x < i.par.x || e.x >= i.par.y {
            discard;
            return vec4f(0.0);
        }
        let large = (i.misc.x & M_CLS_L) != 0u;
        let em = select(CLS_S_EM, CLS_L_EM, large);
        let range = select(CLS_S_RANGE, CLS_L_RANGE, large);
        c = glyph_cov(e - i.nb.zw, i.misc.z, i.misc.w, i.par.zw, inv, em, range);
        if (i.misc.x & (M_PREVG | M_NEXTG)) != 0u {
            // (a pixel and a half around a neighbour's ink box, in em: more than the pixel's reach)
            let mg = 1.5 * (length(inv[0]) + length(inv[1]));
            if (i.misc.x & M_PREVG) != 0u {
                c += neighbour_cov(e, i.ix.y, large, inv, mg);
            }
            if (i.misc.x & M_NEXTG) != 0u {
                c += neighbour_cov(e, i.ix.z, large, inv, mg);
            }
            c = min(c, 1.0);
        }
    }
    if !(c > 0.0) {
        discard;
        return vec4f(0.0);
    }
    // (the colour unpacked where it is returned, as in full_body)
    let col = unpack4x8unorm(i.misc.y);
    let al = col.w * c;
    return vec4f(vec3f(col.z, col.y, col.x) * al, al);
}

// layers of text alone: every record a glyph or a bar (an insert's record of zeros is never visible)
@vertex
fn vs_text(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> VsFill {
    let l = lists[pc.layer + (ii >> 16u)];
    let j = (ii & 0xffffu) * BATCH + (vi >> 2u);
    if j >= l.y {
        return fill_out();
    }
    let it = item_of(visible[l.x + j], l.x + j);
    let r = pool[it.idx];
    if kind_of_record(r) < K_GLYPH {
        return fill_out();
    }
    return text_vertex(vi, it, r, style_in(r.w.z - 1u, it.ins, it.blk), l.z);
}

@fragment
fn fs_text(i: VsFill) -> @location(0) vec4f {
    return text_body(i);
}

// layers of text among anything else: a text record here, a fill's as vs_fill, any other as vs_full
@vertex
fn vs_text_all(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> VsFill {
    let l = lists[pc.layer + (ii >> 16u)];
    let j = (ii & 0xffffu) * BATCH + (vi >> 2u);
    if j >= l.y {
        return fill_out();
    }
    let it = item_of(visible[l.x + j], l.x + j);
    let r = pool[it.idx];
    let k = kind_of_record(r);
    if k >= K_GLYPH {
        return text_vertex(vi, it, r, style_in(r.w.z - 1u, it.ins, it.blk), l.z);
    }
    if k == K_EDGE || k == K_TRI {
        return fill_vertex(vi, it, r, style_in(r.w.z - 1u, it.ins, it.blk), l.z);
    }
    let s = vertex(vi, it, l.z, true);
    var o = fill_out();
    o.pos = s.pos;
    o.seg = s.seg;
    o.nb = s.nb;
    o.par = s.par;
    o.misc = s.misc;
    o.dash = s.dash;
    return o;
}

@fragment
fn fs_text_all(i: VsFill) -> @location(0) vec4f {
    if (i.misc.x & M_TEXT) != 0u {
        return text_body(i);
    }
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
