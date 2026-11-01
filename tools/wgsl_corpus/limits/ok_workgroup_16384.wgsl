// WGSL "Limits": 16384 bytes of workgroup variables
// expect: valid
var<workgroup> v: array<u32, 4096>;
@compute @workgroup_size(1) fn main() { v[0] = 1u; }
