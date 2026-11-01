// WGSL "Entry Points": every entry point parameter needs @builtin or @location
// expect: error
@fragment fn fs(x: f32) -> @location(0) vec4f { return vec4f(x); }
