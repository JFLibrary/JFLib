// WGSL "Built-in Functions": derivatives only in fragment shaders
// expect: error
@group(0) @binding(0) var<storage, read_write> b: array<f32>;
@compute @workgroup_size(1) fn main() { b[0] = dpdx(b[1]); }
