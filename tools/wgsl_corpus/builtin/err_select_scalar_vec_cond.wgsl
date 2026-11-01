// WGSL "Built-in Functions": select of scalars with a vector condition
// expect: error
fn f() { _ = select(1, 2, vec2(true)); }
