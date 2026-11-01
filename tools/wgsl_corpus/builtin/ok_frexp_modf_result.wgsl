// WGSL "Built-in Functions": frexp and modf return structs (fract / exp, fract / whole)
// expect: valid
fn f(x: f32) -> f32 { let a = frexp(x); let b = modf(x); return a.fract * f32(a.exp) + b.whole + b.fract; }
