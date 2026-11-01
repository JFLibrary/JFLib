// WGSL "Literals": an abstract float literal must be finite (1e400)
// expect: error
const a = 1e400;
