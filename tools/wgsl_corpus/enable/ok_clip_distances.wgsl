// WGSL "Enable Extensions": enable clip_distances: the clip_distances vertex output
// expect: valid
enable clip_distances;
struct V { @builtin(position) p: vec4f, @builtin(clip_distances) c: array<f32, 4> }
@vertex fn vs() -> V { var v: V; v.c[0] = 1.0; return v; }
