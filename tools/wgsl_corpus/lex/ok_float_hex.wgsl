// WGSL "Literals": hex float literals: 0x1p4, 0x1.8p1, 0x.8p0, 0x1p-2f
// expect: valid
const a = 0x1p4;
const b = 0x1.8p1;
const c = 0x.8p0;
const d = 0x1p-2f;
const_assert a == 16.0 && b == 3.0 && c == 0.5 && d == 0.25f;
