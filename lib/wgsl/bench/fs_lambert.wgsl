// fs_lambert: colour * (ambient + (1 - ambient) * max(dot(normalize(n), light), 0)), alpha 1
// (lib/shader/bench.rs, docs/SHADER.md "Benchmark-Vertrag")

struct Camera {
    view_proj: mat4x4f,
    planes: array<vec4f, 6>,
    light_dir: vec3f,
    ambient: f32,
}

@group(0) @binding(0) var<uniform> camera: Camera;

@fragment
fn main(@location(0) normal: vec3f, @location(1) @interpolate(flat) color: vec3f) -> @location(0) vec4f {
    let n = normalize(normal);
    let ndl = max(dot(n, camera.light_dir), 0.0);
    let k = camera.ambient + (1.0 - camera.ambient) * ndl;
    return vec4f(color * k, 1.0);
}
