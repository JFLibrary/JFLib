// WGSL "Pipeline Creation": of an array variable
// expect: pipeline-error
override a: i32 = 5;
@compute @workgroup_size(1) fn main() { var v = array<i32, 3>(); var x = 1; x = v[a]; }
