// WGSL "Template Lists": vec2<vec2<f32>> is no type (a vector of vectors)
// expect: error
fn f() { let v = vec2<vec2<f32>>(); }
