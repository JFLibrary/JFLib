// WGSL "Abstract Numeric Types": 1 + 2.5 is an abstract float
// expect: valid
const a = 1 + 2.5;
const_assert a == 3.5;
