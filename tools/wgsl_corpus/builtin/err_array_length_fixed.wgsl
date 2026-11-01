// WGSL "Built-in Functions": arrayLength only of runtime-sized arrays
// expect: error
var<private> a: array<f32, 4>;
fn f() -> u32 { return arrayLength(&a); }
