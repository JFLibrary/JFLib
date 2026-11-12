// cull: the visible lists of a frame, in draw order, in three passes (docs/RENDER2D.md "Sichtbarkeit").  The records
// lie in pages of 256; the pages of the visible layers, in their draw order, are the entries of `einfo` (scene.rs builds
// it), one workgroup each: a layer's pages, and where an insert of a block sits in its layer the pages of the block's
// records for that insert (an entry covers the records `first` to `first + count` of its page and names the insert's
// instance; block.wgsl).  The lists must keep the painter's order (layer order, then the order of insertion, a block's
// entities in theirs within each insert): a translucent stroke drawn before the one under it would blend wrongly.  So no
// atomic counter per list as in render3d, but a stable compaction:
//
//   instances  (G6b2, before the others) per instance of a block: its space for this frame (where the camera lies in the
//           block's units, the map to pixels) and whether its block's box, grown by its strokes' reach, meets the target
//   count   every record of a page against the target (an insert's record through its instance's space, none of an
//           instance out of view): a bit per record (`masks`, 8 words per entry), the page's count; a fill's edge: its
//           segment in pixels to `epx`
//   scan    one workgroup: the exclusive sum of the counts in entry order (`offsets`), then per visible layer its first
//           place and its count (`lists`) and its indirect draw (`draws`): 6 indices per quad, at most BATCH quads per
//           instance (u16 indices), as many instances as that takes; first_instance names the layer on Voll
//   write   every visible record to its place: the page's offset plus the bits before its own (an insert's record as
//           ITEM_BLOCK | its place in the pool, its instance beside it in `vinst`: the vertex shader loads both at once)
//
// The test of a record (shown): the box around its segment, grown by the stroke's reach (half the width times its
// join's bound, reach_bound, where it has a neighbour, else half the width; and a pixel for the smoothed edge) and by
// what f32 may round points far away to, against the target's rectangle, by separating axes.  An arc's piece: the
// box around its chord (its start from the record before it), grown as well by how far the piece bulges beyond the
// chord (its sagitta).  A fill's edge: its box grown by its quad's reach (2 px); a fill's triangle: the box of its three
// corners and 2 px.  Free records and arcs' heads (style 0) never pass; with culling off every live record does.  An
// insert's records take its style where they take theirs by block (block.wgsl style_in), widths in units scaled with it.

const BATCH: u32 = 16384u;

struct Push {
    // entries of `einfo` (the instance pass: instances), visible layers, 1 on Voll (one multi-draw: first_instance = layer
    // << 16)
    entries: u32,
    layers: u32,
    multi: u32,
    reserved: u32,
}

@group(0) @binding(0) var<uniform> view: View;
@group(0) @binding(1) var<storage, read> pool: array<Record>;
@group(0) @binding(2) var<storage, read> styles: array<Style>;
// per entry: the page it looks at, its instance + 1 (0: a layer's page), its records (first | count << 16), the offset of
// its fill edges in epx (block.wgsl)
@group(0) @binding(3) var<storage, read> einfo: array<vec4u>;
// per entry: 8 words, a bit per record of its page
@group(0) @binding(4) var<storage, read_write> masks: array<u32>;
// per entry: its visible records
@group(0) @binding(5) var<storage, read_write> counts: array<u32>;
// per visible layer, in draw order: its first entry, its entries, its transparency (percent, G6b2)
@group(0) @binding(6) var<storage, read> layers: array<vec4u>;
// per entry: the place of its first visible record
@group(0) @binding(7) var<storage, read_write> offsets: array<u32>;
// per visible layer: its first place in `visible`, its count, its transparency (stroke.wgsl reads them)
@group(0) @binding(8) var<storage, read_write> lists: array<vec4u>;
// per visible layer: VkDrawIndexedIndirectCommand (5 words)
@group(0) @binding(9) var<storage, read_write> draws: array<u32>;
@group(0) @binding(10) var<storage, read_write> visible: array<u32>;
// per record: a fill edge's segment in pixels of the target (fill.wgsl reads it; count writes it for every edge of the
// visible layers, in view or not)
@group(0) @binding(11) var<storage, read_write> epx: array<vec4f>;
// per instance of a block: its space this frame (the instance pass writes it, count and the vertex shaders read it)
@group(0) @binding(12) var<storage, read_write> spaces: array<Space>;
// per instance: its transform and box (scene.rs instance words, uploaded when an insert changes)
@group(0) @binding(13) var<storage, read> insts: array<Inst>;
// per visible record of an insert: its instance (write writes it, the vertex shaders read it)
@group(0) @binding(14) var<storage, read_write> vinst: array<u32>;
var<immediate> pc: Push;

var<workgroup> wmask: array<atomic<u32>, 8>;
var<workgroup> partial: array<u32, 256>;

// record i is in the frame: live, and its stroke's box meets the target (see above); vw: the view, or its insert's space
// (blk, its style by block ins)
fn shown(i: u32, vw: View, blk: bool, ins: u32) -> bool {
    let r = pool[i];
    if r.w.z == 0u {
        return false;
    }
    if view.misc.x != 0.0 {
        return true;
    }
    let st = style_in(r.w.z - 1u, ins, blk);
    let kind = kind_of_record(r);
    if kind == K_TRI {
        // a fill's triangle: the box of its corners (the starts of the edge records they name) and 2 px, against the target
        let base = i - bitcast<u32>(r.a.x);
        let t0 = to_px(rel2(pool[base + bitcast<u32>(r.a.y)].a, vw).xy, vw);
        let t1 = to_px(rel2(pool[base + bitcast<u32>(r.a.z)].a, vw).xy, vw);
        let t2 = to_px(rel2(pool[base + bitcast<u32>(r.a.w)].a, vw).xy, vw);
        let m = 2.0 + 1e-3 + 4e-6 * (length(t0) + length(t1) + length(t2));
        let lo = min(min(t0, t1), t2) - vec2f(m);
        let hi = max(max(t0, t1), t2) + vec2f(m);
        let hv = 0.5 * view.turn.zw;
        return all(lo <= hv) && all(hi >= -hv);
    }
    let fill_edge = kind == K_EDGE;
    var h = half_width(st, vw);
    if fill_edge {
        // (a fill's edge: its smoothed edge reaches FILL_R on either side, its quad a pixel more)
        h = 1.0;
    }
    let arc = kind == K_ARC;
    // (an arc piece: from its start, the end of the record before it, to its end)
    var pa = r.a;
    if arc {
        pa = pool[i - 1u].b;
    }
    let p0 = to_px(rel2(pa, vw).xy, vw);
    let p1 = to_px(rel2(r.b, vw).xy, vw);
    // (a miter's bound only where the record has a neighbour: a lone segment's caps reach half its width)
    let joined = (r.w.w & (F_PREV | F_NEXT)) != 0u && !arc && !fill_edge;
    var bulge = 0.0;
    if arc {
        bulge = sagitta(0.5 * length(p1 - p0), (bitcast<f32>(r.v.x) + bitcast<f32>(r.v.y)) * vw.rest.z);
        // (a square cap follows the tangent at its end, which turns up to ARC_STEP / 2 away from the chord: its corner
        // reaches h (cos + sin) of that along and across it, below 1.18 h)
        if cap_of(st) == CAP_SQUARE {
            bulge += 0.18 * h;
        }
    }
    // (f32 holds 24 bits of a point far away: what that may cost, many times over)
    let reach = h * select(1.0, reach_bound(st), joined) + bulge + 1.0 + 1e-3 + 4e-6 * (length(p0) + length(p1));
    let half = 0.5 * view.turn.zw;
    let dv = p1 - p0;
    let len = length(dv);
    var d = vec2f(1.0, 0.0);
    if len > 0.0 {
        d = dv / len;
    }
    let n = vec2f(-d.y, d.x);
    let c = 0.5 * (p0 + p1);
    let e = 0.5 * len + reach;
    if abs(c.x) > half.x + e * abs(d.x) + reach * abs(n.x) {
        return false;
    }
    if abs(c.y) > half.y + e * abs(d.y) + reach * abs(n.y) {
        return false;
    }
    if abs(dot(c, d)) > e + half.x * abs(d.x) + half.y * abs(d.y) {
        return false;
    }
    if abs(dot(c, n)) > reach + half.x * abs(n.x) + half.y * abs(n.y) {
        return false;
    }
    return true;
}

@compute @workgroup_size(256)
fn count(@builtin(local_invocation_index) li: u32, @builtin(workgroup_id) wid: vec3u) {
    let w = wid.x;
    let ei = einfo[w];
    // (the records of the page this entry covers, `pack` instances of them one after the other; an insert's only while
    // its instance is in view: no early return before the barrier, every invocation goes on)
    let cnt = entry_count(ei);
    let pack = entry_pack(ei);
    let k = li / cnt;
    let i = ei.x * PAGE + (ei.z & 255u) + (li - k * cnt);
    var on = k < pack;
    var vw = view;
    var blk = false;
    var ins = 0u;
    var mir = false;
    if ei.y != 0u {
        let sp = spaces[ei.y - 1u + min(k, pack - 1u)];
        vw = space_view(sp);
        blk = true;
        ins = sp.u.y;
        mir = (sp.u.x & SP_MIRROR) != 0u;
        on = on && (sp.u.x & SP_VISIBLE) != 0u;
    }
    let r = pool[i];
    if on && kind_of_record(r) == K_EDGE && r.w.z != 0u {
        // (a mirrored insert's edges turned around: the inside stays on their right in pixels, fill.wgsl)
        var e = edge_px(rel2(r.a, vw), rel2(r.b, vw), vw);
        if mir {
            e = e.zwxy;
        }
        epx[i + ei.w] = e;
    }
    if on && shown(i, vw, blk, ins) {
        atomicOr(&wmask[li >> 5u], 1u << (li & 31u));
    }
    workgroupBarrier();
    if li < 8u {
        masks[w * 8u + li] = atomicLoad(&wmask[li]);
    }
    if li == 0u {
        var n = 0u;
        for (var k = 0u; k < 8u; k++) {
            n += countOneBits(atomicLoad(&wmask[k]));
        }
        counts[w] = n;
    }
}

@compute @workgroup_size(256)
fn scan(@builtin(local_invocation_index) li: u32) {
    let n = pc.entries;
    let per = (n + 255u) / 256u;
    let start = min(li * per, n);
    let end = min(start + per, n);
    var sum = 0u;
    for (var e = start; e < end; e++) {
        sum += counts[e];
    }
    partial[li] = sum;
    workgroupBarrier();
    // the 256 sums, inclusive (Hillis-Steele)
    for (var off = 1u; off < 256u; off = off << 1u) {
        var v = 0u;
        if li >= off {
            v = partial[li - off];
        }
        workgroupBarrier();
        partial[li] += v;
        workgroupBarrier();
    }
    var run = partial[li] - sum;
    for (var e = start; e < end; e++) {
        offsets[e] = run;
        run += counts[e];
    }
    let total = partial[255];
    // (the offsets of every invocation before the layers read them)
    storageBarrier();
    for (var k = li; k < pc.layers; k += 256u) {
        let l = layers[k];
        var first = total;
        if l.x < n {
            first = offsets[l.x];
        }
        var last = total;
        if l.x + l.y < n {
            last = offsets[l.x + l.y];
        }
        let c = last - first;
        lists[k] = vec4u(first, c, l.z, 0u);
        draws[k * 5u] = 6u * min(c, BATCH);
        draws[k * 5u + 1u] = (c + BATCH - 1u) / BATCH;
        draws[k * 5u + 2u] = 0u;
        draws[k * 5u + 3u] = 0u;
        draws[k * 5u + 4u] = select(0u, k << 16u, pc.multi != 0u);
    }
}

@compute @workgroup_size(256)
fn write(@builtin(local_invocation_index) li: u32, @builtin(workgroup_id) wid: vec3u) {
    let w = wid.x;
    let word = masks[w * 8u + (li >> 5u)];
    let bit = li & 31u;
    if ((word >> bit) & 1u) == 0u {
        return;
    }
    var at = offsets[w];
    for (var k = 0u; k < (li >> 5u); k++) {
        at += countOneBits(masks[w * 8u + k]);
    }
    at += countOneBits(word & ((1u << bit) - 1u));
    let ei = einfo[w];
    let cnt = entry_count(ei);
    let k = li / cnt;
    let i = ei.x * PAGE + (ei.z & 255u) + (li - k * cnt);
    if ei.y != 0u {
        visible[at] = ITEM_BLOCK | i;
        vinst[at] = ei.y - 1u + k;
    } else {
        visible[at] = i;
    }
}

// ---------------------------------------------------------------- the instances of blocks (G6b2)

// an instance of a block (scene.rs Xf::words): its transform world = t + M local, t as three f32 per axis, the inverse
// of M as three f32 per entry, M itself; the block's box (local, as points: hi.x, hi.y, lo.x, lo.y); how far its
// strokes reach beyond it (px, block units); flags, the style by block, the first row of its hatches
struct Inst {
    // t.x as three f32, t.y's first
    t: vec4f,
    // t.y's other two, the insert's scale for widths (hi, lo: block.wgsl Space.sw)
    t2: vec4f,
    // M^-1: n00 (three f32), n01 (three), n10 (three), n11 (three)
    n0: vec4f,
    n1: vec4f,
    n2: vec4f,
    // M (f32): m00, m01, m10, m11
    m: vec4f,
    blo: vec4f,
    bhi: vec4f,
    // px, block units
    reach: vec4f,
    // SP_MIRROR, the style by block, the hatch table's first row, what is added to a record's place for its fill edge in epx
    u: vec4u,
}

// three f32 whose sum is a + b + c, each about below half an ulp of the one before
fn renorm3(a: f32, b: f32, c: f32) -> vec3f {
    let x = two_sum(b, c);
    let y = two_sum(a, x.x);
    let z = two_sum(y.y, x.y);
    return vec3f(y.x, z.x, z.y);
}

// sums and products of three f32 to about 2^-70: where the camera lies in a block's units must be as precise as the
// camera's own three f32, whatever the insert's place and scale (docs/RENDER2D.md "Blöcke")
fn t3_add(a: vec3f, b: vec3f) -> vec3f {
    let s = two_sum(a.x, b.x);
    let t = two_sum(a.y, b.y);
    let u = two_sum(s.y, t.x);
    return renorm3(s.x, u.x, (t.y + u.y) + (a.z + b.z));
}

fn t3_mul(a: vec3f, b: vec3f) -> vec3f {
    let p = two_prod(a.x, b.x);
    let q = two_prod(a.x, b.y);
    let r = two_prod(a.y, b.x);
    let s1 = two_sum(q.x, r.x);
    let s2 = two_sum(p.y, s1.x);
    return renorm3(p.x, s2.x, ((s1.y + s2.y) + (q.y + r.y)) + ((a.x * b.z + a.y * b.y) + a.z * b.x));
}

@compute @workgroup_size(64)
fn instances(@builtin(global_invocation_id) gid: vec3u) {
    let k = gid.x;
    if k >= pc.entries {
        return;
    }
    let it = insts[k];
    // the camera's centre minus t, then through M^-1: the camera in the block's units, three f32 per axis
    let dx = t3_add(vec3f(view.center.x, view.center.z, view.rest.x), -vec3f(it.t.x, it.t.y, it.t.z));
    let dy = t3_add(vec3f(view.center.y, view.center.w, view.rest.y), -vec3f(it.t.w, it.t2.x, it.t2.y));
    let cx = t3_add(t3_mul(it.n0.xyz, dx), t3_mul(vec3f(it.n0.w, it.n1.x, it.n1.y), dy));
    let cy = t3_add(t3_mul(vec3f(it.n1.z, it.n1.w, it.n2.x), dx), t3_mul(it.n2.yzw, dy));
    // the map to pixels: the camera's rotation times M over the insert's scale (a rotation, mirrored or not, where the
    // insert scales evenly); pixels per block unit: that scale over the camera's
    let sw = it.t2.z + it.t2.w;
    let c = view.lin;
    let m = it.m;
    var sp: Space;
    sp.center = vec4f(cx.x, cy.x, cx.y, cy.y);
    sp.rest = vec4f(cx.z, cy.z, sw * view.rest.z, view.rest.w / sw);
    sp.lin = vec4f(c.x * m.x + c.y * m.z, c.x * m.y + c.y * m.w, c.z * m.x + c.w * m.z, c.z * m.y + c.w * m.w) / sw;
    sp.sw = vec4f(it.t2.z, it.t2.w, 0.0, 0.0);
    sp.u = vec4u(it.u.x & SP_MIRROR, it.u.y, it.u.z, it.u.w);
    // in view: the box's corners from the camera in the block's units (as rel2 places points), grown by the reach and by
    // what f32 may round far corners to, against the target
    var vis = view.misc.x != 0.0;
    if !vis {
        let vw = space_view(sp);
        let p0 = to_px(rel2(it.blo, vw).xy, vw);
        let p1 = to_px(rel2(it.bhi, vw).xy, vw);
        let p2 = to_px(rel2(vec4f(it.bhi.x, it.blo.y, it.bhi.z, it.blo.w), vw).xy, vw);
        let p3 = to_px(rel2(vec4f(it.blo.x, it.bhi.y, it.blo.z, it.bhi.w), vw).xy, vw);
        let reach = it.reach.x + it.reach.y * sp.rest.z + 2.0 + 1e-3 + 4e-6 * (length(p0) + length(p1) + length(p2) + length(p3));
        let lo = min(min(p0, p1), min(p2, p3)) - vec2f(reach);
        let hi = max(max(p0, p1), max(p2, p3)) + vec2f(reach);
        let hv = 0.5 * view.turn.zw;
        vis = all(lo <= hv) && all(hi >= -hv);
    }
    if vis {
        sp.u.x |= SP_VISIBLE;
    }
    spaces[k] = sp;
}
