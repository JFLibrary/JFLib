// WGSL "Arithmetic Expressions": concrete u32 subtraction below 0 in a const expression wraps
// expect: valid
const a = 0u - 1u;
