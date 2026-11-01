// WGSL "Entry Points": vertex writes vec4f, fragment reads vec4i at the same location
// expect: pipeline-error
struct V { @builtin(position) p: vec4f, @location(0) c: vec4f }
@vertex fn vs() -> V { return V(vec4f(), vec4f()); }
@fragment fn fs(@location(0) @interpolate(flat) d: vec4i) -> @location(0) vec4f { return vec4f(d); }
