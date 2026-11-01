// WGSL "Built-in Functions": dot of integer vectors
// expect: valid
fn f() -> i32 { return dot(vec3i(1), vec3i(2)); }
