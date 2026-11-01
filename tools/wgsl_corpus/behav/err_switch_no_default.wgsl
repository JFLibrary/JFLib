// WGSL "Control Flow": a switch needs a default clause
// expect: error
fn f(x: i32) { switch x { case 1: {} } }
