// WGSL "Statements Behavior Analysis": a loop without break never falls through
// expect: valid
fn f() -> i32 { loop { return 1; } }
