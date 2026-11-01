// WGSL "Literals": true and false are the bool literals
// expect: valid
const a = true;
const b = !false;
const_assert a == b;
