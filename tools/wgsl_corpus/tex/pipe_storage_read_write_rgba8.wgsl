// WGSL "Texture Built-in Functions": read_write rgba8unorm: WGSL takes it, the core WebGPU device refuses the format (bind group layout)
// expect: pipeline-error
@group(0) @binding(0) var st: texture_storage_2d<rgba8unorm, read_write>;
@compute @workgroup_size(1) fn main() { textureStore(st, vec2i(0), textureLoad(st, vec2i(1))); }
