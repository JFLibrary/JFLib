// WGSL "Attributes": an integer location without @interpolate(flat)
// expect: error
struct V { @builtin(position) p: vec4f, @location(0) k: u32 }
@vertex fn vs() -> V { return V(vec4f(), 1u); }
