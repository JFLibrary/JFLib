// WGSL "Texture Built-in Functions": the textureGather component must be a const expression
// expect: error
@group(0) @binding(0) var t: texture_2d<f32>;
@group(0) @binding(1) var s: sampler;

@fragment fn fs(@location(0) @interpolate(flat) c: i32) -> @location(0) vec4f { return textureGather(c, t, s, vec2f()); }
