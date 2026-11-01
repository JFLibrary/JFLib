// WGSL "Texture Built-in Functions": bgra8unorm storage: WGSL takes it, a device without bgra8unorm-storage refuses it at pipeline creation
// expect: pipeline-error
@group(0) @binding(0) var st: texture_storage_2d<bgra8unorm, write>;
@compute @workgroup_size(1) fn main() { textureStore(st, vec2i(0), vec4f()); }
