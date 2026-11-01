// WGSL "Arithmetic Expressions": i32 min / -1 overflows
// expect: error
const a = (-2147483647i - 1i) / -1i;
