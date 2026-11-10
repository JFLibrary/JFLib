// cull: the visible lists of a frame, in draw order, in three passes (docs/RENDER2D.md "Sichtbarkeit").  The records
// lie in pages of 256; the pages of the visible layers, in their draw order, are the entries of `order` (lib.rs builds
// it), one workgroup each.  The lists must keep the painter's order (layer order, then the order of insertion): a
// translucent stroke drawn before the one under it would blend wrongly.  So no atomic counter per list as in render3d,
// but a stable compaction:
//
//   count   every record of a page against the target: a bit per record (`masks`, 8 words per entry), the page's count
//   scan    one workgroup: the exclusive sum of the counts in entry order (`offsets`), then per visible layer its first
//           place and its count (`lists`) and its indirect draw (`draws`): 6 indices per quad, at most BATCH quads per
//           instance (u16 indices), as many instances as that takes; first_instance names the layer on Voll
//   write   every visible record to its place: the page's offset plus the bits before its own
//
// The test of a record (shown): the box around its segment, grown by the stroke's reach (half the width times its
// join's bound, reach_bound, where it has a neighbour, else half the width; and a pixel for the smoothed edge) and by
// what f32 may round points far away to,
// against the target's rectangle, by separating axes.  Free records (style 0) never pass; with culling off every
// live record does.

const BATCH: u32 = 16384u;

struct Push {
    // entries of `order`, visible layers, 1 on Voll (one multi-draw: first_instance = layer << 16)
    entries: u32,
    layers: u32,
    multi: u32,
    reserved: u32,
}

@group(0) @binding(0) var<uniform> view: View;
@group(0) @binding(1) var<storage, read> pool: array<Record>;
@group(0) @binding(2) var<storage, read> styles: array<Style>;
// per entry: the page it looks at
@group(0) @binding(3) var<storage, read> order: array<u32>;
// per entry: 8 words, a bit per record of its page
@group(0) @binding(4) var<storage, read_write> masks: array<u32>;
// per entry: its visible records
@group(0) @binding(5) var<storage, read_write> counts: array<u32>;
// per visible layer, in draw order: its first entry, its entries
@group(0) @binding(6) var<storage, read> layers: array<vec4u>;
// per entry: the place of its first visible record
@group(0) @binding(7) var<storage, read_write> offsets: array<u32>;
// per visible layer: its first place in `visible`, its count (stroke.wgsl reads them)
@group(0) @binding(8) var<storage, read_write> lists: array<vec4u>;
// per visible layer: VkDrawIndexedIndirectCommand (5 words)
@group(0) @binding(9) var<storage, read_write> draws: array<u32>;
@group(0) @binding(10) var<storage, read_write> visible: array<u32>;
var<immediate> pc: Push;

var<workgroup> wmask: array<atomic<u32>, 8>;
var<workgroup> partial: array<u32, 256>;

// a record is in the frame: live, and its stroke's box meets the target (see above)
fn shown(r: Record) -> bool {
    if r.info.x == 0u {
        return false;
    }
    if view.misc.x != 0.0 {
        return true;
    }
    let st = styles[r.info.x - 1u];
    let h = half_width(st, view);
    let r0 = rel2(r.a, view);
    let r1 = rel2(r.b, view);
    let p0 = to_px(r0.xy, view);
    let p1 = to_px(r1.xy, view);
    // (f32 holds 24 bits of a point far away: what that may cost, many times over)
    // (a miter's bound only where the record has a neighbour: a lone segment's caps reach half its width)
    let joined = any(r.near != vec4f(0.0));
    let reach = h * select(1.0, reach_bound(st), joined) + 1.0 + 1e-3 + 4e-6 * (length(p0) + length(p1));
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
    let i = order[w] * PAGE + li;
    if shown(pool[i]) {
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
        lists[k] = vec4u(first, c, 0u, 0u);
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
    visible[at] = order[w] * PAGE + li;
}
