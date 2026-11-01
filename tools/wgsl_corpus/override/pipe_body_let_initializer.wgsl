// WGSL "Pipeline Creation": in a let
// expect: pipeline-error
override a: i32 = 0;
@compute @workgroup_size(1) fn main() { let x = 1 / a; }
