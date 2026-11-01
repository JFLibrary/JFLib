// WGSL "Literals": decimal float literal forms: 1., .5, 1e3, 1.5e-3, 1f, 0f
// expect: valid
const a = 1.;
const b = .5;
const c = 1e3;
const d = 1.5e-3;
const e = 1f;
const f = 0f;
const_assert a == 1.0 && b == 0.5 && c == 1000.0 && e == 1.0f && f == 0.0f;
