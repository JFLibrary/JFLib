// WGSL "Pipeline Creation": clamp with low > high and a runtime e
// expect: pipeline-error
override a: i32 = 5;
@compute @workgroup_size(1) fn main() { var x = 1; x = clamp(x, a, 0); }
