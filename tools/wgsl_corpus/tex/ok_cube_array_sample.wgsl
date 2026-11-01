// WGSL "Texture Built-in Functions": texture_cube_array with textureSampleLevel
// expect: valid
@group(0) @binding(0) var c: texture_cube_array<f32>;
@group(0) @binding(1) var s: sampler;
@fragment fn fs() -> @location(0) vec4f { return textureSampleLevel(c, s, vec3f(1.0, 0.0, 0.0), 1, 0.0); }
