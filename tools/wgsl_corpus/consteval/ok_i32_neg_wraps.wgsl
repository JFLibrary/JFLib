// WGSL "Arithmetic Expressions": negating the smallest concrete i32 wraps to itself
// expect: valid
const a = -(-2147483647i - 1i);
