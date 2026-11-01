// WGSL "Literals": an f32 literal must not overflow f32
// expect: error
const a = 1e39f;
