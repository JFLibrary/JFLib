// WGSL "Texture Built-in Functions": the texel offset is a const expression
// expect: error
@group(0) @binding(0) var t: texture_2d<f32>;
@group(0) @binding(1) var s: sampler;

@fragment fn fs(@location(0) uv: vec2f, @location(1) @interpolate(flat) o: vec2i) -> @location(0) vec4f { return textureSample(t, s, uv, o); }
