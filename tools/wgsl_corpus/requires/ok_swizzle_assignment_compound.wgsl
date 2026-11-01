// WGSL "Language Extensions": compound assignment to a swizzle (swizzle_assignment)
// expect: valid
fn f() -> vec4f { var v = vec4f(1.0); v.wx *= vec2f(2.0); return v; }
