// WGSL "Texture Built-in Functions": textureNumLevels not of storage textures
// expect: error
@group(0) @binding(0) var st: texture_storage_2d<r32float, write>;
fn f() -> u32 { return textureNumLevels(st); }
