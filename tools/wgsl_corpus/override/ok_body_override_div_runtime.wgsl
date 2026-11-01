// WGSL "Pipeline Creation": a / x is decided at run time
// expect: valid
override a: i32 = 0;
@compute @workgroup_size(1) fn main() { var x = 1; x = a / x; }
