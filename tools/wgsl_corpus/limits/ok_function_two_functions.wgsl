// WGSL "Limits": per function
// expect: valid
fn h() { var v: array<u32, 2000>; v[0] = 1u; }
@compute @workgroup_size(1) fn main() { var v: array<u32, 2000>; v[0] = 1u; h(); }
