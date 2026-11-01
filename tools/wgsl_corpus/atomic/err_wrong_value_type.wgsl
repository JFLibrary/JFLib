// WGSL "Atomic Built-in Functions": atomicAdd(&atomic<u32>, i32)
// expect: error
var<workgroup> a: atomic<u32>;
@compute @workgroup_size(1) fn main() { atomicAdd(&a, 1i); }
