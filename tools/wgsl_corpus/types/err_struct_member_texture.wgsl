// WGSL "Pointer Types": a texture is no struct member
// expect: error
struct S { t: texture_2d<f32> }
