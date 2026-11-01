// WGSL "Subgroups": not in vertex shaders
// expect: error
enable subgroups;
@vertex fn vs() -> @builtin(position) vec4f { let a = subgroupAdd(1.0f); return vec4f(a); }
