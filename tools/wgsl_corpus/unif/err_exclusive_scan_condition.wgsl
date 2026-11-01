// WGSL "Uniformity Analysis": and an exclusive one
// expect: error
// uniformity
enable subgroups;
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  if subgroupExclusiveMul(1u) == 0u { let a = subgroupAdd(1u); }
}
