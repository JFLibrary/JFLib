// WGSL "Literals": a tiny f32 literal rounds towards zero instead of failing
// expect: valid
const a = 1e-50f;
