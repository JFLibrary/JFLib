// WGSL "Pipeline Creation": in an argument of a call
// expect: pipeline-error
override a: i32 = 0;
fn g(x: i32) -> i32 { return x; }
@compute @workgroup_size(1) fn main() { var x = g(1 / a); }
