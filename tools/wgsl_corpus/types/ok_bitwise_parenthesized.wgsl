// WGSL "Structure Types": parenthesized, they do
// expect: valid
fn f(a: u32, b: u32, c: u32) -> u32 { return a | (b & c); }
