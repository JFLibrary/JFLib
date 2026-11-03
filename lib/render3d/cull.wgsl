// cull: one view's visibility.  Every instance whose bounding sphere is not completely outside one of
// the view's six planes goes into the list of its mesh: visible[list_first[l] + n] = instance, with n
// from draws[l].instance_count, which is then the list's indirect draw (docs/RENDER3D.md).  A free slot
// has radius -f32::MAX, so its first plane test rejects it and no shader line is spent on it; with
// pc.all (Scene::set_culling(false)) it is the only test.

struct View {
    view_proj: mat4x4f,
    planes: array<vec4f, 6>,
    eye: vec4f,
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
    count: u32,
    all: u32,
}

@group(0) @binding(0) var<uniform> view: View;
@group(0) @binding(1) var<storage, read> spheres: array<vec4f>;
@group(0) @binding(2) var<storage, read> list_of: array<u32>;
@group(0) @binding(3) var<storage, read> list_first: array<u32>;
@group(0) @binding(4) var<storage, read_write> draws: array<DrawCmd>;
@group(0) @binding(5) var<storage, read_write> visible: array<u32>;
var<immediate> pc: Push;

@compute @workgroup_size(256)
fn main(@builtin(global_invocation_id) gid: vec3u) {
    let i = gid.x;
    if i >= pc.count {
        return;
    }
    let s = spheres[i];
    if pc.all != 0u {
        if s.w < 0.0 {
            return;
        }
    } else {
        let nr = -s.w;
        for (var k = 0u; k < 6u; k++) {
            let p = view.planes[k];
            if dot(p.xyz, s.xyz) + p.w < nr {
                return;
            }
        }
    }
    let l = list_of[i];
    let n = atomicAdd(&draws[l].instance_count, 1u);
    visible[list_first[l] + n] = i;
}
