// WGSL "Conversion Expressions": bitcast in a const expression
// expect: valid
const_assert bitcast<u32>(1.0f) == 0x3f800000u;
