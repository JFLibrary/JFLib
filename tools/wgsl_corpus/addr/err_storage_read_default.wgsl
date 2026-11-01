// WGSL "Address Spaces": var<storage> defaults to read: a store is an error
// expect: error
@group(0) @binding(0) var<storage> b: array<f32>;
@compute @workgroup_size(1) fn main() { b[0] = 1.0; }
