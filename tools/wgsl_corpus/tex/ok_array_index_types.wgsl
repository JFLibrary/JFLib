// WGSL "Texture Built-in Functions": array texture layer as i32 or u32
// expect: valid
@group(0) @binding(0) var ta: texture_2d_array<f32>;
@group(0) @binding(1) var s: sampler;
@fragment fn fs() -> @location(0) vec4f { return textureSample(ta, s, vec2f(), 1) + textureSample(ta, s, vec2f(), 2u); }
