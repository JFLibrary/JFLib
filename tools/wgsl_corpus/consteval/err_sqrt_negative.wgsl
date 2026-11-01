// WGSL "Built-in Functions": sqrt(-1.0) in a const expression is an error (no NaN)
// expect: error
const a = sqrt(-1.0);
