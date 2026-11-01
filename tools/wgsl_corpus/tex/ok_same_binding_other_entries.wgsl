// WGSL "Texture Built-in Functions": one group and binding for two resources used by different entry points
// expect: valid
@group(0) @binding(0) var t: texture_2d<f32>;
@group(0) @binding(0) var u: texture_2d<u32>;
@fragment fn a() -> @location(0) vec4f { return textureLoad(t, vec2i(0), 0); }
@fragment fn b() -> @location(0) vec4u { return textureLoad(u, vec2i(0), 0); }
