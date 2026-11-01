// WGSL "Literals": in 0x1.8f the f is a hex digit, not a suffix (an abstract float)
// expect: valid
const a = 0x1.8f;
const_assert a > 1.5 && a < 1.6;
