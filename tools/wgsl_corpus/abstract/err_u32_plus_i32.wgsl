// WGSL "Abstract Numeric Types": 1u + 1i: concrete types never convert
// expect: error
fn f() { let a = 1u + 1i; }
