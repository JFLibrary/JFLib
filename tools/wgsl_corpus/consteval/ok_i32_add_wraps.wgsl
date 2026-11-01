// WGSL "Arithmetic Expressions": concrete i32 addition in a const expression wraps modulo 2^32
// expect: valid
const a = 2147483647i + 1;
