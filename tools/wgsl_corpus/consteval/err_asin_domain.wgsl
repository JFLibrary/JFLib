// WGSL "Built-in Functions": asin(2.0) in a const expression is an error
// expect: error
const a = asin(2.0);
