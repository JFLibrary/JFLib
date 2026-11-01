// WGSL "Pipeline Creation": not with a runtime x
// expect: valid
override a: f32 = 0.0;
@compute @workgroup_size(1) fn main() { var x = 1.0; x = x / a; }
