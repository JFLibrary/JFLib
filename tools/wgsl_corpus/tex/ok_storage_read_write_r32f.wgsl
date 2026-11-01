// WGSL "Texture Built-in Functions": a read_write r32float storage texture: load and store
// expect: valid
@group(0) @binding(0) var st: texture_storage_2d<r32float, read_write>;
@compute @workgroup_size(1) fn main() { let v = textureLoad(st, vec2i(0)); textureStore(st, vec2i(1), v * 2.0); }
