// WGSL "Control Flow": break if is the last statement of continuing
// expect: error
fn f(c: bool) { var i = 0; loop { continuing { break if c; i++; } } }
