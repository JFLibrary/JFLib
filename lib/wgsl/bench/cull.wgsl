// cull: for i < pc.count, if the bounding sphere of objects[i] is not completely outside one
// of the six planes: visible[atomicAdd(&draw.instance_count, 1u)] = i (lib/shader/bench.rs,
// docs/SHADER.md "Benchmark-Vertrag").  The plane test in the order of the CPU reference
// `visible`: ((n.x * c.x + n.y * c.y) + n.z * c.z) + d < -radius.

struct Object {
    pos: vec3f,
    scale: f32,
    rot: vec4f,
    color: vec3f,
    radius: f32,
}

struct Camera {
    view_proj: mat4x4f,
    planes: array<vec4f, 6>,
    light_dir: vec3f,
    ambient: f32,
}

// VkDrawIndexedIndirectCommand
struct DrawCmd {
    index_count: u32,
    instance_count: atomic<u32>,
    first_index: u32,
    vertex_offset: i32,
    first_instance: u32,
}

struct Push {
    time: f32,
    count: u32,
    seed: u32,
    extent: f32,
    object_index: u32,
}

@group(0) @binding(0) var<uniform> camera: Camera;
@group(0) @binding(1) var<storage, read> objects: array<Object>;
@group(0) @binding(2) var<storage, read_write> visible: array<u32>;
@group(0) @binding(3) var<storage, read_write> draw: DrawCmd;
var<immediate> pc: Push;

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3u) {
    var inside = true;
    var k = 0u;
    let i = gid.x;
    if i < pc.count {
        let c = objects[i].pos;
        let nr = -objects[i].radius;
        for (; k < 6u; k++) {
            let p = camera.planes[k];
            let dist = ((p.x * c.x + p.y * c.y) + p.z * c.z) + p.w;
            if dist < nr {
                inside = false;
                break;
            }
        }
        if inside {
            let slot = atomicAdd(&draw.instance_count, 1u);
            visible[slot] = i;
        }
    }
}
