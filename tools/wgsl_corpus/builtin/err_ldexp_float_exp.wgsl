// WGSL "Built-in Functions": ldexp takes an integer exponent
// expect: error
fn f() { _ = ldexp(1.0, 2.0); }
