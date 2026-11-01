// WGSL "Pipeline Creation": in an argument of an atomic
// expect: pipeline-error
override a: u32 = 1u;
var<workgroup> w: atomic<u32>;
@compute @workgroup_size(1) fn main() { atomicStore(&w, 1u / (a - 1u)); }
