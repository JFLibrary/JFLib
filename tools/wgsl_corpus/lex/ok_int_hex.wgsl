// WGSL "Literals": hex integer literals, both x cases, with suffixes
// expect: valid
const a = 0x1F;
const b = 0X1fu;
const c = 0xffi;
const_assert a == 31 && b == 31u && c == 255i;
