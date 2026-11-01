// WGSL "Arithmetic Expressions": abstract float overflow in a const expression
// expect: error
const a = 1e308 * 10.0;
