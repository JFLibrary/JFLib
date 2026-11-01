// WGSL "Pipeline Creation": float division by 0 of constants
// expect: pipeline-error
override a: f32 = 0.0;
@compute @workgroup_size(1) fn main() { var x = 1.0; x = 1.0 / a; }
