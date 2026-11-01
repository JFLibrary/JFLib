// WGSL "Texture Built-in Functions": textureSampleCompare and ...Level on a depth texture
// expect: valid
@group(0) @binding(0) var t: texture_depth_2d;
@group(0) @binding(1) var s: sampler_comparison;
@fragment fn fs(@location(0) uv: vec2f) -> @location(0) vec4f { return vec4f(textureSampleCompare(t, s, uv, 0.5) + textureSampleCompareLevel(t, s, uv, 0.5)); }
