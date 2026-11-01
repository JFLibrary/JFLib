// WGSL "Abstract Numeric Types": abstract 1 << 62 fits 64 bits
// expect: valid
const a = 1 << 62;
const_assert a > 0;
