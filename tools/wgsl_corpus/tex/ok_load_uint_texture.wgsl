// WGSL "Texture Built-in Functions": textureLoad of a texture_2d<u32>
// expect: valid
@group(0) @binding(0) var t: texture_2d<u32>;
@fragment fn fs() -> @location(0) vec4u { return textureLoad(t, vec2i(0), 0); }
