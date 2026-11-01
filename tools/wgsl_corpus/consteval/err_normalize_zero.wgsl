// WGSL "Built-in Functions": normalize of a zero vector in a const expression
// expect: error
const a = normalize(vec2(0.0, 0.0));
