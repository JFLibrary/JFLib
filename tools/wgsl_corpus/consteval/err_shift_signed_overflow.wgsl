// WGSL "Arithmetic Expressions": shifting a 1 into the sign bit of an i32 constant
// expect: error
const a = 1i << 31u;
