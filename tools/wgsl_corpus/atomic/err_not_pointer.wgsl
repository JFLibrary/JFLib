// WGSL "Atomic Built-in Functions": atomic built-ins take a pointer
// expect: error
var<workgroup> a: atomic<u32>;
@compute @workgroup_size(1) fn main() { _ = atomicAdd(a, 1u); }
