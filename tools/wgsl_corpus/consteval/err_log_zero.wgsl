// WGSL "Built-in Functions": log(0.0) in a const expression is an error
// expect: error
const a = log(0.0);
