// WGSL "Identifiers": a module-scope function may take the name of a built-in function
// expect: valid
fn max(a: f32) -> f32 { return a * 2.0; }
fn g() -> f32 { return max(1.0); }
