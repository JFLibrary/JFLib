// cull: one view's visibility.  Every instance whose bounding sphere is not completely outside one of
// the view's six planes goes into the list of its mesh: visible[lists[l].x + n] = instance, with n
// from draws[l].instance_count, which is then the list's indirect draw (docs/RENDER3D.md).  A free slot
// has radius -f32::MAX, so its first plane test rejects it and no shader line is spent on it; with
// pc.all (Scene::set_culling(false)) it is the only test.
//
// (G5d) Detail levels: a list whose mesh has them names them (lists[l].y, .z: the first, how many; each a threshold
// and a list, u32::MAX: not drawn); an instance takes the last one whose threshold its projected size (the sphere's
// diameter over the view's height, from the camera: view.lod) is below.
//
// (G5d) The camera's occlusion test in two phases (docs/RENDER3D.md "Verdeckung"): `early` takes what phase 2 of
// the frame before found visible (a bit per slot in `hidden`, 0: visible or changed since), `late` tests every
// instance in the view against the depth pyramid of what phase 1 drew (hiz.wgsl), adds what is visible and phase 1
// did not draw, and writes the bits for the next frame.  `main` is the plain test (shadow cascades, no occlusion).

struct View {
    view_proj: mat4x4f,
    planes: array<vec4f, 6>,
    eye: vec4f,
    // (G5d) clip x = proj.x * x, clip y = proj.y * y, depth = proj.z + proj.w / z in the camera's space (z along
    // forward)
    proj: vec4f,
    // (G5d) the camera's eye (relative to the origin) and fy: a sphere of radius r at distance d has the size
    // r fy / d (the same in every view)
    lod: vec4f,
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
    // (G5d) late: the target's size in pixels, the pyramid's levels
    w: u32,
    h: u32,
    levels: u32,
}

@group(0) @binding(0) var<uniform> view: View;
@group(0) @binding(1) var<storage, read> spheres: array<vec4f>;
@group(0) @binding(2) var<storage, read> list_of: array<u32>;
// per list: its first place in `visible`, its detail levels' first entry and number; then the levels: threshold
// (f32 bits), list
@group(0) @binding(3) var<storage, read> lists: array<vec4u>;
@group(0) @binding(4) var<storage, read_write> draws: array<DrawCmd>;
@group(0) @binding(5) var<storage, read_write> visible: array<u32>;
// (G5d) bit i & 31 of word i >> 5: slot i was hidden in phase 2 of the frame before
@group(0) @binding(6) var<storage, read_write> hidden: array<u32>;
// (G5d) the depth pyramid of this frame's phase 1 (late only): mip m holds level m + 1
@group(1) @binding(0) var pyramid: texture_2d<f32>;
var<immediate> pc: Push;

// (G5d) the bits of a workgroup's 256 slots, put together before they are written
var<workgroup> hid: array<atomic<u32>, 8>;

// the sphere is not completely outside one of the six planes
fn in_view(s: vec4f) -> bool {
    let nr = -s.w;
    for (var k = 0u; k < 6u; k++) {
        let p = view.planes[k];
        if dot(p.xyz, s.xyz) + p.w < nr {
            return false;
        }
    }
    return true;
}

const NONE: u32 = 0xffffffffu;

// (G5d) the list of slot i with sphere s: its mesh's, or that of the detail level its size picks (NONE: not drawn)
fn pick(i: u32, s: vec4f) -> u32 {
    var l = list_of[i];
    let info = lists[l];
    if info.z > 0u && view.lod.w > 0.0 {
        let size = s.w * view.lod.w / max(distance(s.xyz, view.lod.xyz), 1e-30);
        for (var k = 0u; k < info.z; k++) {
            let e = lists[info.y + k];
            if size >= bitcast<f32>(e.x) {
                break;
            }
            l = e.y;
        }
    }
    return l;
}


// slot i into list l
fn add(i: u32, l: u32) {
    let n = atomicAdd(&draws[l].instance_count, 1u);
    visible[lists[l].x + n] = i;
}

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
        add(i, list_of[i]);
        return;
    }
    if !in_view(s) {
        return;
    }
    let l = pick(i, s);
    if l != NONE {
        add(i, l);
    }
}

// (G5d) phase 1: what the frame before saw (or what changed since), in the view
@compute @workgroup_size(256)
fn early(@builtin(global_invocation_id) gid: vec3u) {
    let i = gid.x;
    if i >= pc.count {
        return;
    }
    if ((hidden[i >> 5u] >> (i & 31u)) & 1u) != 0u {
        return;
    }
    let s = spheres[i];
    if in_view(s) {
        let l = pick(i, s);
        if l != NONE {
            add(i, l);
        }
    }
}

// (G5d) the sphere lies behind everything the pyramid holds over its rectangle on the screen (false when in doubt):
// its nearest depth (reverse Z: the largest) below the smallest depth of the rectangle.  The rectangle is the
// projection of the box around the sphere in the camera's space, a pixel wider on each side; the level is the first
// whose texels make it at most 2 x 2 of them
fn occluded(s: vec4f) -> bool {
    let c = view.view_proj * vec4f(s.xyz, 1.0);
    // larger by what f32 rounds in the centre, the eye and the matrix
    let big = max(max(max(abs(s.x), abs(s.y)), abs(s.z)), max(max(abs(view.eye.x), abs(view.eye.y)), abs(view.eye.z)));
    let r = s.w * 1.0001 + big * 4e-6;
    let zn = c.w - r;
    // the near plane cuts it, or it reaches behind the eye: not tested
    if !(zn > 0.0) {
        return false;
    }
    let dn = view.proj.z + view.proj.w / zn;
    if !(dn < 1.0) {
        return false;
    }
    let zf = c.w + r;
    let rx = view.proj.x * r;
    let ry = view.proj.y * r;
    let x0 = min((c.x - rx) / zn, (c.x - rx) / zf);
    let x1 = max((c.x + rx) / zn, (c.x + rx) / zf);
    let y0 = min((c.y - ry) / zn, (c.y - ry) / zf);
    let y1 = max((c.y + ry) / zn, (c.y + ry) / zf);
    // pixels from the top left (NDC y up), clamped before they become integers
    let size = vec2f(f32(pc.w), f32(pc.h));
    let lo = clamp(vec2f(x0 * 0.5 + 0.5, 0.5 - y1 * 0.5) * size, vec2f(-2.0), size + 2.0);
    let hi = clamp(vec2f(x1 * 0.5 + 0.5, 0.5 - y0 * 0.5) * size, vec2f(-2.0), size + 2.0);
    let top = vec2i(size) - 1;
    let a = clamp(vec2i(floor(lo)) - 1, vec2i(0), top);
    let b = clamp(vec2i(floor(hi)) + 1, vec2i(0), top);
    // level k: a texel covers 2^k pixels a side; e < 2^k pixels apart are at most two texels
    let e = u32(max(b.x - a.x, b.y - a.y));
    var level = 1u;
    if e > 1u {
        level = firstLeadingBit(e) + 1u;
    }
    if level > pc.levels {
        return false;
    }
    let m = i32(level) - 1;
    let t0 = vec2i(vec2u(a) >> vec2u(level));
    let t1 = vec2i(vec2u(b) >> vec2u(level));
    let far = min(min(textureLoad(pyramid, t0, m).x, textureLoad(pyramid, t1, m).x),
                  min(textureLoad(pyramid, vec2i(t0.x, t1.y), m).x, textureLoad(pyramid, vec2i(t1.x, t0.y), m).x));
    return dn < far;
}

// (G5d) phase 2: every instance in the view against the pyramid; what is visible and phase 1 did not draw goes into
// the lists, and every slot's bit is written for the next frame (a workgroup's 8 words at once: their old values are
// read before the barrier, the new ones written after it)
@compute @workgroup_size(256)
fn late(@builtin(global_invocation_id) gid: vec3u, @builtin(local_invocation_index) li: u32, @builtin(workgroup_id) wid: vec3u) {
    let i = gid.x;
    var hide = true;
    if i < pc.count {
        let s = spheres[i];
        let drawn = ((hidden[i >> 5u] >> (i & 31u)) & 1u) == 0u;
        if in_view(s) {
            let l = pick(i, s);
            if l != NONE {
                hide = occluded(s);
                if !hide && !drawn {
                    add(i, l);
                }
            }
        }
    }
    if hide {
        atomicOr(&hid[li >> 5u], 1u << (li & 31u));
    }
    workgroupBarrier();
    storageBarrier();
    let w = wid.x * 8u + li;
    if li < 8u && w < arrayLength(&hidden) {
        hidden[w] = atomicLoad(&hid[li]);
    }
}
