// WGSL "Literals": a decimal float literal with a dot may have leading zeros
// expect: valid
const a = 01.5;
const_assert a == 1.5;
