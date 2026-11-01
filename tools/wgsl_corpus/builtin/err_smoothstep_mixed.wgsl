// WGSL "Built-in Functions": smoothstep(T, T, T): no scalar edges for vector x
// expect: error
fn f() { _ = smoothstep(0.0, 1.0, vec2f()); }
