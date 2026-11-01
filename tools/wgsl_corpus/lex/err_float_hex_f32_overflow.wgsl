// WGSL "Literals": 0x1p128f is 2^128, beyond f32
// expect: error
const a = 0x1p128f;
