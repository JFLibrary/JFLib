// WGSL "Subgroups": subgroupElect takes nothing
// expect: error
enable subgroups;
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  let a = subgroupElect(1u);
}
