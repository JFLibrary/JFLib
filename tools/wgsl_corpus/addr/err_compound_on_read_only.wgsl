// WGSL "Address Spaces": a compound assignment needs a writable reference
// expect: error
@group(0) @binding(0) var<storage, read> b: array<u32>;
@compute @workgroup_size(1) fn main() { b[0] += 1u; }
