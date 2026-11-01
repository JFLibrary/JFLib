// WGSL "Abstract Numeric Types": 4294967295 converts to u32
// expect: valid
fn f() -> u32 { let a: u32 = 4294967295; return a; }
