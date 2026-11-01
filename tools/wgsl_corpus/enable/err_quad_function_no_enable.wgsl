// WGSL "Subgroups": quadSwapX too
// expect: error
@compute @workgroup_size(64) fn main() { let a = quadSwapX(1u); }
