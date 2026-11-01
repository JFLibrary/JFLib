// WGSL "Array Types": the count may be u32, i32 or abstract
// expect: valid
fn f() { var a: array<i32, 4u>; var b: array<i32, 4i>; var c: array<i32, 2 + 2>; a = b; b = c; }
