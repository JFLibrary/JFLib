// WGSL "Literals": the abstract -2147483648 converts to i32
// expect: valid
const a: i32 = -2147483648;
const_assert a == -2147483647i - 1i;
