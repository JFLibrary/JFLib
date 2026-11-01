// WGSL "Limits": over: a pipeline error (16-byte steps)
// expect: pipeline-error
var<workgroup> v: array<u32, 4097>;
@compute @workgroup_size(1) fn main() { v[0] = 1u; }
