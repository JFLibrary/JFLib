// WGSL "Language Extensions": requires swizzle_assignment: v.xz = ...
// expect: valid
requires swizzle_assignment;
fn f() -> vec3f { var v = vec3f(); v.xz = vec2f(1.0, 2.0); v.y += 3.0; return v; }
