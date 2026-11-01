// WGSL "Literals": a hex abstract integer literal must fit i64 (0x8000000000000000 does not)
// expect: error
const a = 0x8000000000000000;
