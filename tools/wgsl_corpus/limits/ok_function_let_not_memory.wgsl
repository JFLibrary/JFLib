// WGSL "Limits": a let is no memory
// expect: valid
@compute @workgroup_size(1) fn main() { let v = array<u32, 5000>(); _ = v[0]; }
