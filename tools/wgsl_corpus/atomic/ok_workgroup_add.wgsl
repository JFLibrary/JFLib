// WGSL "Atomic Built-in Functions": atomicAdd on a workgroup atomic (abstract operand)
// expect: valid
var<workgroup> a: atomic<u32>;
@compute @workgroup_size(8) fn main() { _ = atomicAdd(&a, 1); }
