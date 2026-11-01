// WGSL "Pipeline Creation": 1u << a with a >= 32
// expect: pipeline-error
override a: u32 = 40u;
@compute @workgroup_size(1) fn main() { var x = 1u; x = 1u << a; }
