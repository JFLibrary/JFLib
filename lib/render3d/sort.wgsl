// sort: (G7f) the blended instances back to front (docs/RENDER3D.md "Durchsichtiges").  cull.wgsl put every blended
// instance in view into one list with a key, the bits of !(distance squared from the eye to its sphere's centre): sorted
// ascending by key, then by slot (a tie decides the same way in every frame), the farthest comes first.  A bitonic sort
// over the list's room, a power of two and a multiple of TILE (Scene keeps it so); the places from the count on are
// treated as the largest key (PAD) and end behind the instances.
//
//   sort_local, pc.k == 0: each tile of TILE places sorted in workgroup memory (the stages up to TILE; a tile's direction
//                          by its place, so two neighbours form the bitonic sequence of the next stage)
//   sort_global(k, j):     one step of stage k at a distance j >= TILE over the whole list, a pair per invocation
//   sort_local, pc.k > 0:  the steps of stage k at distances below TILE, a tile in workgroup memory
// Scene: sort_local(0), then for k = 2 TILE .. n: sort_global(k, j) for j = k / 2 .. TILE, sort_local(k).

struct BlendList {
    vertex_count: u32,
    count: u32,
    first_vertex: u32,
    first_instance: u32,
    items: array<vec4u>,
}

struct Push {
    // the stage (0: the tiles from nothing), the step's distance (sort_global), the list's room
    k: u32,
    j: u32,
    n: u32,
}

@group(0) @binding(0) var<storage, read_write> blend: BlendList;
var<immediate> pc: Push;

const TILE: u32 = 512u;
const PAD: vec4u = vec4u(0xffffffffu, 0xffffffffu, 0u, 0u);
var<workgroup> tile: array<vec4u, 512>;

// a comes before b: the smaller key, then the smaller slot
fn before(a: vec4u, b: vec4u) -> bool {
    return a.x < b.x || (a.x == b.x && a.y < b.y);
}

// the places of pair p in a step of distance j (a power of two): i, i + j
fn pair(p: u32, j: u32) -> vec2u {
    let i = ((p & ~(j - 1u)) << 1u) | (p & (j - 1u));
    return vec2u(i, i | j);
}

@compute @workgroup_size(256)
fn sort_local(@builtin(local_invocation_index) li: u32, @builtin(workgroup_id) wid: vec3u) {
    let base = wid.x * TILE;
    let count = blend.count;
    for (var e = li; e < TILE; e += 256u) {
        let g = base + e;
        var v = blend.items[g];
        if pc.k == 0u && g >= count {
            v = PAD;
        }
        tile[e] = v;
    }
    workgroupBarrier();
    var k = 2u;
    var last = TILE;
    if pc.k != 0u {
        k = pc.k;
        last = pc.k;
    }
    for (; k <= last; k <<= 1u) {
        for (var j = min(k >> 1u, TILE >> 1u); j > 0u; j >>= 1u) {
            // TILE / 2 pairs: one per invocation
            let ij = pair(li, j);
            let up = ((base + ij.x) & k) == 0u;
            let a = tile[ij.x];
            let b = tile[ij.y];
            if before(b, a) == up {
                tile[ij.x] = b;
                tile[ij.y] = a;
            }
            workgroupBarrier();
        }
    }
    for (var e = li; e < TILE; e += 256u) {
        blend.items[base + e] = tile[e];
    }
}

@compute @workgroup_size(256)
fn sort_global(@builtin(global_invocation_id) gid: vec3u) {
    let p = gid.x;
    if p >= pc.n / 2u {
        return;
    }
    let ij = pair(p, pc.j);
    let up = (ij.x & pc.k) == 0u;
    let a = blend.items[ij.x];
    let b = blend.items[ij.y];
    if before(b, a) == up {
        blend.items[ij.x] = b;
        blend.items[ij.y] = a;
    }
}
