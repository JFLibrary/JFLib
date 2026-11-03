// scatter: the frame's changed instances, each to its slot in the three arrays.  One buffer, one
// dispatch, however scattered the slots are: k changes as k copy commands (or k calls of writeBuffer)
// would cost far more (docs/RENDER3D.md).

struct Instance {
    m0: vec4f,
    m1: vec4f,
    m2: vec4f,
    normal_scale: vec3f,
    material: u32,
}

struct Change {
    slot: u32,
    list: u32,
    reserved0: u32,
    reserved1: u32,
    sphere: vec4f,
    instance: Instance,
}

struct Push {
    count: u32,
}

@group(0) @binding(0) var<storage, read> changes: array<Change>;
@group(0) @binding(1) var<storage, read_write> spheres: array<vec4f>;
@group(0) @binding(2) var<storage, read_write> list_of: array<u32>;
@group(0) @binding(3) var<storage, read_write> instances: array<Instance>;
var<immediate> pc: Push;

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) gid: vec3u) {
    let i = gid.x;
    if i >= pc.count {
        return;
    }
    let c = changes[i];
    spheres[c.slot] = c.sphere;
    list_of[c.slot] = c.list;
    instances[c.slot] = c.instance;
}
