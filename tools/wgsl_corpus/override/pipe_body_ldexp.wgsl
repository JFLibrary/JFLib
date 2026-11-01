// WGSL "Pipeline Creation": ldexp with a too large exponent
// expect: pipeline-error
override a: i32 = 500;
@compute @workgroup_size(1) fn main() { var x = 1.0; x = ldexp(1.0, a); }
