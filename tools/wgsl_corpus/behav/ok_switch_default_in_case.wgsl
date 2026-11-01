// WGSL "Control Flow": default may share a clause with case values
// expect: valid
fn f(x: i32) -> i32 { switch x { case 1, default, 2: { return 1; } } }
