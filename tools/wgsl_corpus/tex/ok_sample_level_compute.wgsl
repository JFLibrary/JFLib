// WGSL "Texture Built-in Functions": textureSampleLevel in a compute shader
// expect: valid
@group(0) @binding(0) var t: texture_2d<f32>;
@group(0) @binding(1) var s: sampler;

@group(0) @binding(2) var<storage, read_write> o: vec4f;
@compute @workgroup_size(1) fn main() { o = textureSampleLevel(t, s, vec2f(0.5), 0.0); }
