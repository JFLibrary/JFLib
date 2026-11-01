// WGSL "Control Flow": a for loop with all three parts
// expect: valid
fn f() -> i32 { var s = 0; for (var i = 0; i < 4; i++) { s += i; } return s; }
