// WGSL "Built-in Functions": clamp(T, T, T): scalars do not splat to a vector
// expect: error
fn f() { _ = clamp(vec2f(), 0.0, 1.0); }
