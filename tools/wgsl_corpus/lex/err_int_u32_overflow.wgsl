// WGSL "Literals": a u32 literal must fit u32
// expect: error
const a = 4294967296u;
