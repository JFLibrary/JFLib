// WGSL "Pipeline Creation": in an argument of select
// expect: pipeline-error
override a: i32 = 0;
@compute @workgroup_size(1) fn main() { var x = 1; x = select(1, 1 / a, false); }
