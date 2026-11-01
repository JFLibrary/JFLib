// WGSL "Pipeline Creation": not on the right side of a decided short-circuit
// expect: valid
override a: i32 = 0;
override t: bool = false;
@compute @workgroup_size(1) fn main() { var b = t && (1 / a == 0); }
