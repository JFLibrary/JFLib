// WGSL "Control Flow": a case selector twice
// expect: error
fn f(x: i32) { switch x { case 1, 1: {} default: {} } }
