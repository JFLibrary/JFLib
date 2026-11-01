// WGSL "Identifiers": once shadowed, the built-in overload is gone
// expect: error
fn max(a: f32) -> f32 { return a; }
fn g() -> f32 { return max(1.0, 2.0); }
