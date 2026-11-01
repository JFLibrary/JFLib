// WGSL "Pipeline Creation": a float overflow
// expect: pipeline-error
override a: f32 = 1e38;
@compute @workgroup_size(1) fn main() { var x = 1.0; x = a * 100.0; }
