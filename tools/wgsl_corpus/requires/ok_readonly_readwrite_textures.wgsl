// WGSL "Language Extensions": requires readonly_and_readwrite_storage_textures
// expect: valid
requires readonly_and_readwrite_storage_textures;
@group(0) @binding(0) var st: texture_storage_2d<r32uint, read_write>;
@compute @workgroup_size(1) fn main() { textureStore(st, vec2i(0), textureLoad(st, vec2i(1)) + vec4u(1u)); }
