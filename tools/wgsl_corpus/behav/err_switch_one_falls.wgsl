// WGSL "Statements Behavior Analysis": a switch clause that falls through
// expect: error
fn f(x: i32) -> i32 { switch x { case 1: { return 1; } default: {} } }
