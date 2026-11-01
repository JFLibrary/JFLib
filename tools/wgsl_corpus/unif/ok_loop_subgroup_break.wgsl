// WGSL "Uniformity Analysis": a loop with a subgroup-uniform break
// expect: valid
// uniformity
enable subgroups;
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  loop { let a = subgroupAdd(1u); if subgroupAny(true) { break; } }
}
