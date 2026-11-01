// WGSL "Limits": also through a helper
// expect: error
var<private> v: array<u32, 2049>;
fn h() { v[0] = 1u; }
@compute @workgroup_size(1) fn main() { h(); }
