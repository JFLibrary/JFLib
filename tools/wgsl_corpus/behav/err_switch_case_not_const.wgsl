// WGSL "Control Flow": case values are const expressions
// expect: error
fn f(x: i32, y: i32) { switch x { case y: {} default: {} } }
