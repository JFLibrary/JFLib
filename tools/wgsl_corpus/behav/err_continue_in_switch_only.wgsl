// WGSL "Control Flow": continue in a switch that is not inside a loop
// expect: error
fn f(x: i32) { switch x { default: { continue; } } }
