// WGSL "Enable Extensions": clip_distances has at most 8 elements
// expect: error
enable clip_distances;
struct V { @builtin(position) p: vec4f, @builtin(clip_distances) c: array<f32, 9> }
@vertex fn vs() -> V { var v: V; return v; }
