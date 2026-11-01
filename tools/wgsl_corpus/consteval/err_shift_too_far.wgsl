// WGSL "Arithmetic Expressions": a const shift by the bit width or more
// expect: error
const a = 1u << 32u;
