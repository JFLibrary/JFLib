// WGSL "Control Flow": the loop with a break if
// expect: valid
fn f() -> i32 { var i = 0; loop { continuing { i++; break if i >= 4; } } return i; }
