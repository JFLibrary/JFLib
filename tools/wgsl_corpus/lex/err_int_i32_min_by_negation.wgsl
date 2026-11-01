// WGSL "Literals": -2147483648i negates an out-of-range literal: still an error
// expect: error
const a = -2147483648i;
