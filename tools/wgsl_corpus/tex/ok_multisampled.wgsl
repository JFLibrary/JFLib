// WGSL "Texture Built-in Functions": textureLoad with a sample index of a multisampled texture
// expect: valid
@group(0) @binding(0) var t: texture_multisampled_2d<f32>;
@group(0) @binding(1) var d: texture_depth_multisampled_2d;
@fragment fn fs() -> @location(0) vec4f { return textureLoad(t, vec2i(0), 1) + textureLoad(d, vec2i(0), 0); }
