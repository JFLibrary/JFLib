// WGSL "Language Extensions": var<immediate> of 80 bytes exceeds maxImmediateSize (64)
// expect: pipeline-error
requires immediate_address_space;
struct C { m: mat4x4f, k: vec4f }
var<immediate> c: C;
@vertex fn vs(@location(0) p: vec4f) -> @builtin(position) vec4f { return c.m * p + c.k; }
