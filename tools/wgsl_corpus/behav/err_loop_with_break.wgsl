// WGSL "Statements Behavior Analysis": a loop with break falls through
// expect: error
fn f() -> i32 { loop { break; } }
