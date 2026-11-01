// WGSL "Texture Built-in Functions": textureLoad has no cube overload
// expect: error
@group(0) @binding(0) var t: texture_cube<f32>;
@fragment fn fs() -> @location(0) vec4f { return textureLoad(t, vec3i(0), 0); }
