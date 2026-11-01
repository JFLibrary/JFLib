// WGSL "Pipeline Creation": with another value it is fine
// expect: valid
// constants: {"a": 2}
override a: i32 = 0;
@compute @workgroup_size(1) fn main() { var x = 1; x = 1 / a; }
