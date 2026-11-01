// WGSL "Entry Points": the fragment reads a location the vertex shader does not write
// expect: pipeline-error
struct V { @builtin(position) p: vec4f, @location(0) c: vec4f }
@vertex fn vs() -> V { return V(vec4f(), vec4f()); }
@fragment fn fs(@location(1) d: vec4f) -> @location(0) vec4f { return d; }
