// WGSL "Array Types": a negative element count
// expect: error
fn f() { var a: array<i32, -1>; }
