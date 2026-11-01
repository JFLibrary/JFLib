// WGSL "Literals": 4294967295u is the largest u32 literal
// expect: valid
const a = 4294967295u;
const b = 0xffffffffu;
const_assert a == b;
