// WGSL "Built-in Functions": smoothstep with low == high in a const expression
// expect: error
const a = smoothstep(1.0, 1.0, 0.5);
