// WGSL "Texture Built-in Functions": textureGather with a const component 0 .. 3
// expect: valid
@group(0) @binding(0) var t: texture_2d<f32>;
@group(0) @binding(1) var s: sampler;

@fragment fn fs() -> @location(0) vec4f { return textureGather(1, t, s, vec2f()); }
