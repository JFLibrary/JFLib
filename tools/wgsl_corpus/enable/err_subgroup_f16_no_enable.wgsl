// WGSL "Subgroups": f16 still needs enable f16
// expect: error
enable subgroups;
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  let a = subgroupAdd(1h);
}
