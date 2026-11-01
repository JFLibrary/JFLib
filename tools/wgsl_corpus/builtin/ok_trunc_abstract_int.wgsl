// WGSL "Built-in Functions": trunc(1): the abstract int converts to an abstract float
// expect: valid
const_assert trunc(1) == 1.0;
