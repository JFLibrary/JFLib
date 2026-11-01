// WGSL "Built-in Functions": const built-ins: sqrt, abs, clamp, countOneBits, dot
// expect: valid
const_assert sqrt(4.0) == 2.0;
const_assert abs(-5) == 5;
const_assert clamp(5, 0, 3) == 3;
const_assert countOneBits(0xffu) == 8u;
const_assert dot(vec2(1, 2), vec2(3, 4)) == 11;
