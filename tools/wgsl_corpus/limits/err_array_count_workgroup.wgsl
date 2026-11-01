// WGSL "Limits": in a workgroup variable (not a pipeline error)
// expect: error
var<workgroup> v: array<u32, 65536>;
