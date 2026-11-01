// WGSL "Structure Types": shifts do not chain (a << b << c)
// expect: error
fn f(a: u32) -> u32 { return a << 1u << 2u; }
