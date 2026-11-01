// WGSL "Statements Behavior Analysis": behavior analysis ignores constant conditions: if (true) still falls through
// expect: error
fn f() -> i32 { if true { return 1; } }
