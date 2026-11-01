// WGSL "Limits": 8196: over the spec's minimum (Chrome has no such limit)
// expect: error
@compute @workgroup_size(1) fn main() { var v: array<u32, 2049>; v[0] = 1u; }
