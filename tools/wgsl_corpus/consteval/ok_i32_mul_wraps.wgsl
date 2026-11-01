// WGSL "Arithmetic Expressions": concrete i32 multiplication in a const expression wraps
// expect: valid
const a = 65536i * 65536i;
