// WGSL "const_assert Statement": a false const_assert is a shader-creation error
// expect: error
const_assert 1 > 2;
