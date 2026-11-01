// WGSL "Literals": an i32 literal must fit i32: 2147483648i is out of range
// expect: error
const a = 2147483648i;
