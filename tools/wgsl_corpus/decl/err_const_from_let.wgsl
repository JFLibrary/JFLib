// WGSL "Value Declarations": a const initializer must be a const expression
// expect: error
fn f(x: i32) { const a = x; }
