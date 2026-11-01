// WGSL "Texture Built-in Functions": textureDimensions, textureNumLayers, textureNumLevels, textureNumSamples
// expect: valid
@group(0) @binding(0) var a: texture_2d_array<f32>;
@group(0) @binding(1) var m: texture_multisampled_2d<f32>;
@group(0) @binding(2) var st: texture_storage_2d<r32float, write>;
@group(0) @binding(3) var<storage, read_write> o: array<u32>;
@compute @workgroup_size(1) fn main() { o[0] = textureDimensions(a).x + textureDimensions(a, 1).y + textureNumLayers(a) + textureNumLevels(a) + textureNumSamples(m) + textureDimensions(st).x; }
