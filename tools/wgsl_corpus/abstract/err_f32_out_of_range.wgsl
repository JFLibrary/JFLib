// WGSL "Abstract Numeric Types": 1e39 is out of the f32 range
// expect: error
fn f() { let a: f32 = 1e39; }
