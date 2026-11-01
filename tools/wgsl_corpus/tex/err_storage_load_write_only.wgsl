// WGSL "Texture Built-in Functions": no textureLoad of a write-only storage texture
// expect: error
@group(0) @binding(0) var st: texture_storage_2d<rgba8unorm, write>;
@compute @workgroup_size(1) fn main() { _ = textureLoad(st, vec2i(0)); }
