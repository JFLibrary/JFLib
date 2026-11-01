// WGSL "Limits": all variables of a function add up
// expect: error
@compute @workgroup_size(1) fn main() { { var v: array<u32, 1024>; v[0] = 1u; } { var w: array<u32, 1025>; w[0] = 1u; } }
