// WGSL "Literals": an abstract integer literal must fit 64 bits
// expect: error
const a = 9223372036854775808;
