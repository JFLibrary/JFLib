// WGSL "Uniformity Analysis": and with a non-uniform one
// expect: error
// uniformity
enable subgroups;
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  loop { let a = subgroupAdd(1u); if i == 0u { break; } }
}
