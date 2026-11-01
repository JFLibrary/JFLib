// WGSL "Address Spaces": var<function> inside a function
// expect: valid
fn f() -> i32 { var<function> a: i32 = 2; return a; }
