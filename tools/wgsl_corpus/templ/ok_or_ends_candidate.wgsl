// WGSL "Template Lists": || inside a candidate template list ends it
// expect: valid
fn f(a: i32, b: i32, c: i32, d: i32) -> bool { return a < b || c > d; }
