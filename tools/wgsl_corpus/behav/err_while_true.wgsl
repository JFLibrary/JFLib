// WGSL "Statements Behavior Analysis": while may end (it has Next) even with true
// expect: error
fn f() -> i32 { while true { return 1; } }
