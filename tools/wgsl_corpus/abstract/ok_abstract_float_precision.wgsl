// WGSL "Abstract Numeric Types": abstract floats are f64: 0.1 + 0.2 != 0.3
// expect: valid
const_assert 0.1 + 0.2 != 0.3;
