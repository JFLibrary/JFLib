// WGSL "Attributes": @invariant only on the position built-in
// expect: error
struct V { @builtin(position) p: vec4f, @invariant @location(0) c: f32 }
@vertex fn vs() -> V { return V(vec4f(), 1.0); }
