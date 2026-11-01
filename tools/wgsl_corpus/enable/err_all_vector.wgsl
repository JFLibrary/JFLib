// WGSL "Subgroups": subgroupAll only on a scalar
// expect: error
enable subgroups;
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  let a = subgroupAll(vec2<bool>(true));
}
