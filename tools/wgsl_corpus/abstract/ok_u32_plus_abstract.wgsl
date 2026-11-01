// WGSL "Abstract Numeric Types": 1u + 1: the abstract operand converts to u32
// expect: valid
fn f() -> u32 { return 1u + 1; }
