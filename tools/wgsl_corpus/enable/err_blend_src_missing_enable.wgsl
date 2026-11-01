// WGSL "Enable Extensions": @blend_src needs enable dual_source_blending
// expect: error
struct O { @location(0) @blend_src(0) a: vec4f, @location(0) @blend_src(1) b: vec4f }
@fragment fn fs() -> O { return O(vec4f(1.0), vec4f(0.5)); }
