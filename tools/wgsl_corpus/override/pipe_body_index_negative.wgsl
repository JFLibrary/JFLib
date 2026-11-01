// WGSL "Pipeline Creation": a negative index
// expect: pipeline-error
override a: i32 = -1;
@compute @workgroup_size(1) fn main() { var v = array<i32, 3>(); var x = 1; x = v[a]; }
