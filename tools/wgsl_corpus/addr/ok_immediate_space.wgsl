// WGSL "Address Spaces": var<immediate> (immediate_address_space): data set with the pass
// expect: valid
var<immediate> c: vec4f;
@fragment fn fs() -> @location(0) vec4f { return c; }
