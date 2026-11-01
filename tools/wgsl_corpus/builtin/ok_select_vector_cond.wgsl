// WGSL "Built-in Functions": select with a bool vector condition of the same size
// expect: valid
fn f() -> vec2f { return select(vec2f(), vec2f(1.0), vec2(true, false)); }
