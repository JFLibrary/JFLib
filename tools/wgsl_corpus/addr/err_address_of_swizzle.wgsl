// WGSL "Address Spaces": no pointer to a multi-component swizzle
// expect: error
fn f() { var v = vec2f(1.0, 2.0); let p = &v.xy; }
