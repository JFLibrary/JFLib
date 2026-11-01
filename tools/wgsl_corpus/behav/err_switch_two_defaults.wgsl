// WGSL "Control Flow": two default clauses
// expect: error
fn f(x: i32) { switch x { default: {} case 1, default: {} } }
