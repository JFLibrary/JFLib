// WGSL "Matrix Types": mat2x3(...) infers its component type from the columns
// expect: valid
fn f() -> mat2x3f { return mat2x3(vec3(1.0), vec3(2.0)); }
