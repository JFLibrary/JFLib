// WGSL "Texture Built-in Functions": a read-only rgba8unorm storage texture
// expect: valid
@group(0) @binding(0) var st: texture_storage_2d<rgba8unorm, read>;
@group(0) @binding(1) var<storage, read_write> o: vec4f;
@compute @workgroup_size(1) fn main() { o = textureLoad(st, vec2i(0)); }
