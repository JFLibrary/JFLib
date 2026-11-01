// WGSL "Built-in Functions": extractBits with offset + count beyond the width in a const expression
// expect: error
const a = extractBits(1u, 30u, 8u);
