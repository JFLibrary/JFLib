// WGSL "Entry Points": two outputs at one location
// expect: error
struct O { @location(0) a: vec4f, @location(0) b: vec4f }
@fragment fn fs() -> O { return O(vec4f(), vec4f()); }
