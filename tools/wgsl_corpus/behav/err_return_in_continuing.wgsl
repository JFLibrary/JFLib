// WGSL "Control Flow": return is not allowed in continuing
// expect: error
fn f() { loop { continuing { return; } } }
