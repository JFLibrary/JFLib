// WGSL "Built-in Functions": pow(2, 3): abstract ints convert to abstract floats
// expect: valid
const_assert pow(2, 3) == 8.0;
