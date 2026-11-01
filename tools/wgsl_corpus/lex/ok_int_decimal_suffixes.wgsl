// WGSL "Literals": decimal integer literals with i / u suffixes
// expect: valid
const a = 123;
const b = 123i;
const c = 123u;
const d = 0u;
const_assert a == 123 && b == 123i && c == 123u && d == 0u;
