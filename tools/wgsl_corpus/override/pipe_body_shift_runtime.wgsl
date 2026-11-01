// WGSL "Pipeline Creation": x << a
// expect: pipeline-error
override a: u32 = 40u;
@compute @workgroup_size(1) fn main() { var x = 1u; x = x << a; }
