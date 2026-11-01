// WGSL "Attributes": integer user IO needs @interpolate(flat)
// expect: valid
struct V { @builtin(position) p: vec4f, @location(0) @interpolate(flat) k: u32 }
@vertex fn vs() -> V { return V(vec4f(), 1u); }
@fragment fn fs(@location(0) @interpolate(flat) k: u32) -> @location(0) vec4f { return vec4f(f32(k)); }
