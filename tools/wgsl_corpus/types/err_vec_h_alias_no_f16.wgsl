// WGSL "Vector Types": vec2h needs enable f16
// expect: error
fn f() { let v = vec2h(); }
