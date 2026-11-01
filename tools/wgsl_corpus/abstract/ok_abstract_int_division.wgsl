// WGSL "Abstract Numeric Types": 3 / 2 is integer division in the abstract int domain, then converted
// expect: valid
const a: f32 = 3 / 2;
const_assert a == 1.0;
