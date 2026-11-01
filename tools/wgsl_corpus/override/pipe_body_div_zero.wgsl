// WGSL "Pipeline Creation": 1 / a with a = 0 in a function body
// expect: pipeline-error
override a: i32 = 0;
@compute @workgroup_size(1) fn main() { var x = 1; x = 1 / a; }
