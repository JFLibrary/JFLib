// WGSL "Control Flow": case values of the selector type (u32 against i32)
// expect: error
fn f(x: i32) { switch x { case 1u: {} default: {} } }
