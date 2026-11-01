// WGSL "Pipeline Creation": i32 min / -1
// expect: pipeline-error
override a: i32 = -1;
@compute @workgroup_size(1) fn main() { var x = 1; x = -2147483648 / a; }
