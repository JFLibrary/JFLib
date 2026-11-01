// WGSL "Abstract Numeric Types": 0x7fffffff + 1 overflows when the let makes it i32
// expect: error
fn f() { let a = 0x7fffffff + 1; }
