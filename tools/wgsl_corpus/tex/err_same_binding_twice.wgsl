// WGSL "Texture Built-in Functions": two resources at one group and binding in one entry point
// expect: error
@group(0) @binding(0) var t: texture_2d<f32>;
@group(0) @binding(0) var u: texture_2d<f32>;
@fragment fn fs() -> @location(0) vec4f { return textureLoad(t, vec2i(0), 0) + textureLoad(u, vec2i(0), 0); }
