// WGSL "Texture Built-in Functions": texture_external with textureSampleBaseClampToEdge and textureLoad
// expect: valid
@group(0) @binding(0) var e: texture_external;
@group(0) @binding(1) var s: sampler;
@fragment fn fs() -> @location(0) vec4f { return textureSampleBaseClampToEdge(e, s, vec2f()) + textureLoad(e, vec2u(0)); }
