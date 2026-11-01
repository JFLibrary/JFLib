// WGSL "Texture Built-in Functions": a storage texture type needs an access mode
// expect: error
@group(0) @binding(0) var st: texture_storage_2d<rgba8unorm>;
