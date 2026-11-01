// WGSL "Constructible Types": vec2(1i, 2u): no common component type
// expect: error
fn f() { let v = vec2(1i, 2u); }
