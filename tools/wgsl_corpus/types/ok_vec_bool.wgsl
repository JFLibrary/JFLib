// WGSL "Vector Types": vectors of bool exist
// expect: valid
const v = vec4<bool>(true, false, true, false);
const_assert any(v);
