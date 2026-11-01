// WGSL "Atomic Built-in Functions": atomicAdd as a statement (its result may be dropped)
// expect: valid
var<workgroup> a: atomic<u32>;
@compute @workgroup_size(1) fn main() { atomicAdd(&a, 1u); }
