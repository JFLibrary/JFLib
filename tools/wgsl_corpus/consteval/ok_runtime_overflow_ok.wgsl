// WGSL "Arithmetic Expressions": the same overflow at run time wraps (no error)
// expect: valid
fn f(x: i32) -> i32 { return x + 2147483647i; }
