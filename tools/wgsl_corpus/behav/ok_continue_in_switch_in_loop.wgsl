// WGSL "Control Flow": continue in a switch inside a loop
// expect: valid
fn f(x: i32) { loop { switch x { case 1: { continue; } default: { break; } } break; } }
