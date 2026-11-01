// WGSL "Literals": abstract integer literals are 64-bit: 9223372036854775807 fits
// expect: valid
const a = 9223372036854775807;
const_assert a > 0;
