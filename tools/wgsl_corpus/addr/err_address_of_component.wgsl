// WGSL "Address Spaces": no pointer to a vector component: &v.x
// expect: error
fn f() -> f32 { var v = vec2f(1.0, 2.0); let p = &v.x; *p = 3.0; return v.x; }
