// WGSL "Matrix Types": a matrix from column-major scalars (abstract ints convert)
// expect: valid
const m = mat2x2<f32>(1, 2, 3, 4);
const_assert m[1][0] == 3.0;
