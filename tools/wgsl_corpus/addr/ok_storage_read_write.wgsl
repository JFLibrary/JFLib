// WGSL "Address Spaces": var<storage, read_write> may be stored to
// expect: valid
@group(0) @binding(0) var<storage, read_write> b: array<f32>;
@compute @workgroup_size(1) fn main() { b[0] = 1.0; b[1] += 2.0; }
