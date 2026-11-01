// WGSL "Built-in Functions": pow(-1.0, 0.5) in a const expression
// expect: error
const a = pow(-1.0, 0.5);
