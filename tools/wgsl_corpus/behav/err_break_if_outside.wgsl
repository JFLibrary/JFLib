// WGSL "Control Flow": break if only in a continuing block
// expect: error
fn f(c: bool) { loop { break if c; } }
