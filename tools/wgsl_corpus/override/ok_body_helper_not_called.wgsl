// WGSL "Pipeline Creation": in a helper the entry point does not call
// expect: valid
override a: i32 = 0;
fn h() -> i32 { return 1 / a; }
@compute @workgroup_size(1) fn main() { }
