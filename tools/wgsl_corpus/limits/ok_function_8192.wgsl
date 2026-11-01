// WGSL "Limits": 8192 bytes of function variables
// expect: valid
@compute @workgroup_size(1) fn main() { var v: array<u32, 2048>; v[0] = 1u; }
