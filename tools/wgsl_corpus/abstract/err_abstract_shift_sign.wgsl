// WGSL "Abstract Numeric Types": abstract 1 << 63 changes the sign bit: an error
// expect: error
const a = 1 << 63;
