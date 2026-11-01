// WGSL "Pipeline Creation": component-wise
// expect: pipeline-error
override a: i32 = 0;
@compute @workgroup_size(1) fn main() { var x = vec2i(1); x = vec2i(1) / vec2i(a); }
