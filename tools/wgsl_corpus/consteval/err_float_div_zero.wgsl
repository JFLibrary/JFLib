// WGSL "Arithmetic Expressions": a float division giving infinity in a const expression
// expect: error
const a = 1.0 / 0.0;
