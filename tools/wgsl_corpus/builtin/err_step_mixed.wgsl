// WGSL "Built-in Functions": step(T, T): no scalar edge for a vector
// expect: error
fn f() { _ = step(0.5, vec2f()); }
