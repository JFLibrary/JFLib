// WGSL "Uniformity Analysis": an atomic is not subgroup-uniform
// expect: error
// uniformity
enable subgroups;
var<workgroup> a: atomic<u32>;
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  if atomicLoad(&a) == 0u { let b = subgroupAdd(1u); }
}
