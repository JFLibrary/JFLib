// WGSL "Texture Built-in Functions": textureStore into a write-only storage texture
// expect: valid
@group(0) @binding(0) var st: texture_storage_2d<rgba8unorm, write>;
@compute @workgroup_size(1) fn main() { textureStore(st, vec2i(0), vec4f(1.0)); }
