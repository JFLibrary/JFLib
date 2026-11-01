// WGSL "Structure Types": different bitwise operators do not mix without parentheses
// expect: error
fn f(a: u32, b: u32, c: u32) -> u32 { return a | b & c; }
