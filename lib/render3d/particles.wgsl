// particles: (G7f) billboards from a buffer (docs/RENDER3D.md "Partikel"; particles.rs has the CPU's side).  A ring of
// records the CPU wrote at their birth; nothing writes them afterwards.  `cs_particles` evaluates each record's motion at
// its age in closed form (gravity along -y, a linear drag) and puts every living particle into the frame's list (the same
// layout as the blended instances' list: an indirect draw, then key, record, 0, 0 per particle) with the key of its
// distance from the eye, farthest first; sort.wgsl sorts it.  Particles in the viewmodel's space (flag 4) follow the
// camera and come after all the world's (the key's top bit), drawn with the viewmodel's view.  `vs_particle` makes six
// vertices per particle (a quad facing the camera, or stretched along the velocity), `fs_particle` a soft disc in
// premultiplied alpha (additive particles: alpha 0), exposed and tone mapped like the scene.

// (draw.wgsl's) encode the colour to sRGB before it goes to the target
@id(0) override ENCODE_SRGB: bool = false;

// cull.wgsl's View, as far as needed
struct View {
    view_proj: mat4x4f,
    planes: array<vec4f, 6>,
    eye: vec4f,
}

// draw.wgsl's Lights, as far as needed: sun_dir.w the exposure, sun_color.w 1 for the neutral tone mapper
struct Lights {
    sun_dir: vec4f,
    sun_color: vec4f,
}

// a particle as its emitter made it (particles.rs spawn): place and birth (microseconds), velocity and life (seconds),
// sizes, colours at birth and death (halves: r g, b a), gravity and drag (halves), flags (1 additive, 2 a streak, 4 the
// viewmodel's; the streak's stretch in the high half).  64 bytes: no vec2u (its alignment of 8 would pad the struct)
struct Particle {
    pos: vec3f,
    birth: u32,
    vel: vec3f,
    life: f32,
    size: u32,
    c0rg: u32,
    c0ba: u32,
    c1rg: u32,
    c1ba: u32,
    motion: u32,
    flags: u32,
    reserved: u32,
}

// the frame: the clock (microseconds), the records, whether the view is orthographic; the camera (relative to the origin):
// eye, forward (w: the near plane), right and up (the viewmodel's space: x right, y up, -z forward)
struct Frame {
    now: u32,
    count: u32,
    ortho: u32,
    reserved: u32,
    eye: vec4f,
    fwd: vec4f,
    right: vec4f,
    up: vec4f,
}

// (cull.wgsl's BlendList) the particles in this frame's list
struct List {
    vertex_count: u32,
    count: atomic<u32>,
    first_vertex: u32,
    first_instance: u32,
    items: array<vec4u>,
}

struct ListRead {
    vertex_count: u32,
    count: u32,
    first_vertex: u32,
    first_instance: u32,
    items: array<vec4u>,
}

@group(0) @binding(0) var<uniform> frame: Frame;
@group(0) @binding(1) var<storage, read> particles: array<Particle>;
@group(0) @binding(2) var<storage, read_write> list: List;

// the draw's own bindings (group 0 of its layout)
@group(0) @binding(3) var<uniform> view: View;
@group(0) @binding(4) var<uniform> vm_view: View;
@group(0) @binding(5) var<uniform> lights: Lights;
@group(0) @binding(6) var<storage, read> drawn: ListRead;

const VIEWMODEL: u32 = 4u;
const STREAK: u32 = 2u;
const ADDITIVE: u32 = 1u;

// a particle's place and velocity at age t in its own space (the world's relative to the origin, or the camera's)
struct Moved {
    pos: vec3f,
    vel: vec3f,
}

fn motion(p: Particle, t: f32) -> Moved {
    let gd = unpack2x16float(p.motion);
    let acc = vec3f(0.0, -gd.x, 0.0);
    let k = gd.y;
    var m: Moved;
    if k > 1e-4 {
        let e = exp(-k * t);
        let a = acc / k;
        m.vel = a + (p.vel - a) * e;
        m.pos = p.pos + a * t + (p.vel - a) * ((1.0 - e) / k);
    } else {
        m.vel = p.vel + acc * t;
        m.pos = p.pos + p.vel * t + acc * (0.5 * t * t);
    }
    return m;
}

// a vector of the camera's space in the world (the viewmodel's particles)
fn from_camera(v: vec3f) -> vec3f {
    return frame.right.xyz * v.x + frame.up.xyz * v.y - frame.fwd.xyz * v.z;
}

// the particle's age in seconds (a wrapped difference of microseconds)
fn age_of(p: Particle) -> f32 {
    return f32(frame.now - p.birth) * 1e-6;
}

@compute @workgroup_size(64)
fn cs_particles(@builtin(global_invocation_id) gid: vec3u) {
    let i = gid.x;
    if i >= frame.count {
        return;
    }
    let p = particles[i];
    let age = age_of(p);
    // (a place never written has life 0)
    if !(age < p.life) {
        return;
    }
    let m = motion(p, age);
    var w = m.pos;
    let vm = (p.flags & VIEWMODEL) != 0u;
    if vm {
        w = frame.eye.xyz + from_camera(m.pos);
    }
    let d = w - frame.eye.xyz;
    // the farthest first: the inverted bits of the distance squared (an orthographic view: of the depth along forward
    // beyond its near plane), halved so the viewmodel's (top bit set) come after all the world's
    var key = ~bitcast<u32>(dot(d, d));
    if frame.ortho != 0u && !vm {
        key = ~bitcast<u32>(max(dot(d, frame.fwd.xyz) - frame.fwd.w, 0.0));
    }
    key = key >> 1u;
    if vm {
        key = key | 0x80000000u;
    }
    let n = atomicAdd(&list.count, 1u);
    if n < arrayLength(&list.items) {
        list.items[n] = vec4u(key, i, 0u, 0u);
    }
}

struct POut {
    @builtin(position) position: vec4f,
    // the corner, -1 ..= 1 along both axes of the billboard
    @location(0) uv: vec2f,
    @location(1) color: vec4f,
    @location(2) @interpolate(flat) flags: u32,
}

// vertex vi (two triangles) of the particle ii places from the back of the sorted list
@vertex
fn vs_particle(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> POut {
    var out: POut;
    out.position = vec4f(2.0, 2.0, 2.0, 1.0);
    if ii >= arrayLength(&drawn.items) {
        return out;
    }
    let p = particles[drawn.items[ii].y];
    let age = age_of(p);
    let t = clamp(age / p.life, 0.0, 1.0);
    let m = motion(p, age);
    let vm = (p.flags & VIEWMODEL) != 0u;
    var w = m.pos;
    var vel = m.vel;
    if vm {
        w = frame.eye.xyz + from_camera(m.pos);
        vel = from_camera(m.vel);
    }
    let sizes = unpack2x16float(p.size);
    let half = 0.5 * mix(sizes.x, sizes.y, t);
    // the corners (-1, -1), (1, -1), (1, 1), (-1, -1), (1, 1), (-1, 1)
    let k = vi % 6u;
    let cx = select(-1.0, 1.0, k == 1u || k == 2u || k == 4u);
    let cy = select(-1.0, 1.0, k == 2u || k == 4u || k == 5u);
    var ax = frame.right.xyz * half;
    var ay = frame.up.xyz * half;
    if (p.flags & STREAK) != 0u {
        // along the velocity on the screen (its part across the view), longer by speed times the stretch
        let f = frame.fwd.xyz;
        let across = vel - f * dot(vel, f);
        let len = length(across);
        if len > 1e-6 {
            let a = across / len;
            let stretch = unpack2x16float(p.flags).y;
            ax = a * (half + 0.5 * len * stretch);
            ay = cross(f, a) * half;
        }
    }
    let at = vec4f(w + ax * cx + ay * cy, 1.0);
    if vm {
        out.position = vm_view.view_proj * at;
    } else {
        out.position = view.view_proj * at;
    }
    out.uv = vec2f(cx, cy);
    let a = unpack2x16float(p.c0rg);
    let b = unpack2x16float(p.c0ba);
    let c = unpack2x16float(p.c1rg);
    let d = unpack2x16float(p.c1ba);
    out.color = mix(vec4f(a, b), vec4f(c, d), t);
    out.flags = p.flags;
    return out;
}

// (draw.wgsl's) Khronos PBR Neutral
fn neutral(c: vec3f) -> vec3f {
    let x = min(c.x, min(c.y, c.z));
    let offset = select(0.04, x - 6.25 * x * x, x < 0.08);
    var col = c - offset;
    let peak = max(col.x, max(col.y, col.z));
    if peak < 0.76 {
        return col;
    }
    let d = 0.24;
    let new_peak = 1.0 - d * d / (peak + d - 0.76);
    col = col * (new_peak / peak);
    let g = 1.0 - 1.0 / (0.15 * (peak - new_peak) + 1.0);
    return mix(col, vec3f(new_peak), g);
}

// (draw.wgsl's) linear light to sRGB's code values by the definition
fn srgb_encode(c: vec3f) -> vec3f {
    let v = clamp(c, vec3f(0.0), vec3f(1.0));
    return select(1.055 * pow(v, vec3f(1.0 / 2.4)) - 0.055, v * 12.92, v <= vec3f(0.0031308));
}

// a soft disc: opacity 1 - r^2 times the colour's alpha; the colour exposed, tone mapped, encoded, premultiplied (additive:
// alpha 0, the light added)
@fragment
fn fs_particle(f: POut) -> @location(0) vec4f {
    let a = max(1.0 - dot(f.uv, f.uv), 0.0) * clamp(f.color.w, 0.0, 1.0);
    var c = max(f.color.xyz, vec3f(0.0)) * lights.sun_dir.w;
    if lights.sun_color.w != 0.0 {
        c = neutral(c);
    }
    if ENCODE_SRGB {
        c = srgb_encode(c);
    }
    if (f.flags & ADDITIVE) != 0u {
        return vec4f(c * a, 0.0);
    }
    return vec4f(c * a, a);
}
