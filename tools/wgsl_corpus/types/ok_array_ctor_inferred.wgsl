// WGSL "Array Types": array(1, 2.0) infers array<AbstractFloat, 2> and concretizes to f32
// expect: valid
fn f() -> f32 { let a = array(1, 2.0); return a[1]; }
