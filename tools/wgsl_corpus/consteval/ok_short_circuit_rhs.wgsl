// WGSL "Logical Expressions": the right side of a short-circuit && is not evaluated (no division by zero)
// expect: valid
const_assert !(false && (1 / 0 == 1));
