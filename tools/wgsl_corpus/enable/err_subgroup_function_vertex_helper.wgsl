// WGSL "Subgroups": also through a helper
// expect: error
enable subgroups;
fn h() -> f32 { return subgroupAdd(1.0f); }
@vertex fn vs() -> @builtin(position) vec4f { return vec4f(h()); }
