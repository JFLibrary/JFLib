// WGSL "Arithmetic Expressions": abstract 2147483647 + 1 is fine, its conversion to i32 is not
// expect: error
const a: i32 = 2147483647 + 1;
