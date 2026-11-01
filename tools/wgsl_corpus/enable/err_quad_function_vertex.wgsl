// WGSL "Subgroups": quad functions neither
// expect: error
enable subgroups;
@vertex fn vs() -> @builtin(position) vec4f { let a = quadSwapY(1.0f); return vec4f(a); }
