// WGSL "Pipeline Creation": sqrt of a negative value
// expect: pipeline-error
override a: f32 = -1.0;
@compute @workgroup_size(1) fn main() { var x = 1.0; x = sqrt(a); }
