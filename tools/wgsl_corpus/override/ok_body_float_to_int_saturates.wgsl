// WGSL "Pipeline Creation": a float to int conversion saturates
// expect: valid
override a: f32 = 1e30;
@compute @workgroup_size(1) fn main() { var x = 1; x = i32(a); }
