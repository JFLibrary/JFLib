// WGSL "Limits": the variables add up
// expect: error
var<private> v: array<u32, 1024>;
var<private> w: array<u32, 1025>;
@compute @workgroup_size(1) fn main() { v[0] = 1u; w[0] = 1u; }
