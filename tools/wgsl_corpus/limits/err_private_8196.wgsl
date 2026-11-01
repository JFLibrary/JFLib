// WGSL "Limits": 8196: over the spec's minimum (Chrome has no such limit)
// expect: error
var<private> v: array<u32, 2049>;
@compute @workgroup_size(1) fn main() { v[0] = 1u; }
