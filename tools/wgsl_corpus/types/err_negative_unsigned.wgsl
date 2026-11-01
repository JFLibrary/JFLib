// WGSL "Structure Types": unary minus has no u32 overload
// expect: error
fn f(a: u32) -> u32 { return -a; }
