// WGSL "Pipeline Creation": also in a branch that never runs
// expect: pipeline-error
override a: i32 = 0;
@compute @workgroup_size(1) fn main() { if false { var x = 1 / a; } }
