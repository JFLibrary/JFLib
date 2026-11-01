// WGSL "Address Spaces": a workgroup var used by a fragment entry point
// expect: error
var<workgroup> w: f32;
@fragment fn fs() -> @location(0) vec4f { return vec4f(w); }
