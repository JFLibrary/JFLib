// WGSL "Variable Declarations": a private var may have a const initializer
// expect: valid
var<private> a: i32 = 1;
var<private> b = vec2(1.0, 2.0);
