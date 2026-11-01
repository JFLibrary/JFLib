// WGSL "Abstract Numeric Types": select(1, 2.0, c) unifies to abstract float
// expect: valid
fn f(c: bool) -> f32 { return select(1, 2.0, c); }
