// WGSL "Abstract Numeric Types": 1u << 31 is a u32
// expect: valid
fn f() -> u32 { let a = 1u << 31; return a; }
