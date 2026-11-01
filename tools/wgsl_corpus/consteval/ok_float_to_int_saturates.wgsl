// WGSL "Conversion Expressions": i32(1e10) saturates (no error)
// expect: valid
const_assert i32(1e10) == 2147483647;
const_assert u32(-1.5) == 0u;
