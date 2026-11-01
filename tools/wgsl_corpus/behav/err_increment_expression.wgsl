// WGSL "Control Flow": i++ is a statement, not an expression
// expect: error
fn f() { var i = 0; let x = i++; }
