// WGSL "Memory Layout": mat3x3f in storage (column stride 16)
// expect: valid
@group(0) @binding(0) var<storage> m: array<mat3x3f>;
@compute @workgroup_size(1) fn main() { _ = m[0][2]; }
