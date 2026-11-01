// WGSL "Statements Behavior Analysis": a switch whose clauses all return
// expect: valid
fn f(x: i32) -> i32 { switch x { case 1: { return 1; } default: { return 2; } } }
