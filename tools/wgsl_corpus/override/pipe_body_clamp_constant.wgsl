// WGSL "Pipeline Creation": clamp with low > high
// expect: pipeline-error
override a: i32 = 5;
@compute @workgroup_size(1) fn main() { var x = 1; x = clamp(1, a, 0); }
