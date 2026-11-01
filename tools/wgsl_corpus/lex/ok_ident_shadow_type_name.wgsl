// WGSL "Identifiers": predeclared names (f32) are not keywords: a local may shadow them
// expect: valid
fn f() -> i32 { let f32 = 2; return f32; }
