// WGSL "Texture Built-in Functions": rgb8unorm is no texel format
// expect: error
@group(0) @binding(0) var st: texture_storage_2d<rgb8unorm, write>;
