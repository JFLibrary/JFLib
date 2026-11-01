// WGSL "Language Extensions": requires texture_formats_tier1: an r8unorm storage texture
// expect: valid
// features: all
requires texture_formats_tier1;
@group(0) @binding(0) var st: texture_storage_2d<r8unorm, write>;
@compute @workgroup_size(1) fn main() { textureStore(st, vec2i(0), vec4f(0.5)); }
