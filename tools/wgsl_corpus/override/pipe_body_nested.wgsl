// WGSL "Pipeline Creation": nested in a runtime expression
// expect: pipeline-error
override a: i32 = 0;
@compute @workgroup_size(1) fn main() { var x = 1; x = x + (2 * (1 / a)); }
