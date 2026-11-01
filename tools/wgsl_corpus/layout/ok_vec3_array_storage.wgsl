// WGSL "Memory Layout": array<vec3f> in storage: stride 16
// expect: valid
@group(0) @binding(0) var<storage> b: array<vec3f>;
@compute @workgroup_size(1) fn main() { _ = b[1]; }
