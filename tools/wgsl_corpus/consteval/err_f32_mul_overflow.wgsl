// WGSL "Arithmetic Expressions": f32 overflow in a const expression
// expect: error
const a = 3.4e38f * 10.0f;
