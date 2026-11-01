// WGSL "Limits": with an override's value
// expect: pipeline-error
override o = 5000;
var<workgroup> v: array<u32, o>;
@compute @workgroup_size(1) fn main() { v[0] = 1u; }
