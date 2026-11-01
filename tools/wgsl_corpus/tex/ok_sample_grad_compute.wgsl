// WGSL "Texture Built-in Functions": textureSampleGrad works outside fragment shaders
// expect: valid
@group(0) @binding(0) var t: texture_2d<f32>;
@group(0) @binding(1) var s: sampler;

@group(0) @binding(2) var<storage, read_write> o: vec4f;
@compute @workgroup_size(1) fn main() { o = textureSampleGrad(t, s, vec2f(), vec2f(0.1), vec2f(0.1)); }
