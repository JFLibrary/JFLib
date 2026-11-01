// WGSL "Texture Built-in Functions": textureLoad of a depth texture gives f32
// expect: valid
@group(0) @binding(0) var d: texture_depth_2d;
@fragment fn fs() -> @location(0) vec4f { let x: f32 = textureLoad(d, vec2i(0), 0); return vec4f(x); }
