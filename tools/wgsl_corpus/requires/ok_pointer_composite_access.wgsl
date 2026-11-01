// WGSL "Language Extensions": requires pointer_composite_access: p.x and p[i] through a pointer
// expect: valid
requires pointer_composite_access;
struct S { v: vec4f, a: array<f32, 4> }
fn g(p: ptr<function, S>) -> f32 { return p.v.x + p.a[2]; }
fn f() -> f32 { var s: S; return g(&s); }
