// WGSL "Statements Behavior Analysis": both branches return
// expect: valid
fn f(c: bool) -> i32 { if c { return 1; } else { return 2; } }
