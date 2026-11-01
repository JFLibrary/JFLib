// WGSL "Pipeline Creation": a derived expression
// expect: pipeline-error
override a: i32 = 0;
@compute @workgroup_size(1) fn main() { var x = 1; x = 1 / (a + 0); }
