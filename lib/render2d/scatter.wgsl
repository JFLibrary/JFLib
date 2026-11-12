// scatter: the frame's changed records, each to its place in the pool (render3d's pattern: one buffer, one dispatch,
// however scattered the places are).  A free record is all zeros (style 0).

struct Change {
    at: u32,
    r0: u32,
    r1: u32,
    r2: u32,
    rec: Record,
}

struct Push {
    count: u32,
}

@group(0) @binding(0) var<storage, read> changes: array<Change>;
@group(0) @binding(1) var<storage, read_write> pool: array<Record>;
// (common.wgsl's functions name it; this pass uses none of them, so it is no binding of its layout)
@group(0) @binding(2) var<uniform> view: View;
var<immediate> pc: Push;

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) gid: vec3u) {
    let i = gid.x;
    if i >= pc.count {
        return;
    }
    let c = changes[i];
    pool[c.at] = c.rec;
}
