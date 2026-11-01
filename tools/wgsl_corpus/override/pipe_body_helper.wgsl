// WGSL "Pipeline Creation": in a helper
// expect: pipeline-error
override a: i32 = 0;
fn h() -> i32 { return 1 / a; }
@compute @workgroup_size(1) fn main() { _ = h(); }
