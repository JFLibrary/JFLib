// WGSL "Built-in Functions": clamp with low > high in a const expression
// expect: error
const a = clamp(1, 3, 0);
