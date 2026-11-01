// WGSL "Entry Points": @location(16) is beyond maxInterStageShaderVariables (16)
// expect: pipeline-error
struct V { @builtin(position) p: vec4f, @location(16) c: vec4f }
@vertex fn vs() -> V { return V(vec4f(), vec4f()); }
