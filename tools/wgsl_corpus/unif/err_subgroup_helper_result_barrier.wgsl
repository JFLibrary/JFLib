// WGSL "Uniformity Analysis": not for a barrier
// expect: error
// uniformity
enable subgroups;
fn f() -> u32 { return subgroupAdd(1u); }
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  if f() == 0u { workgroupBarrier(); }
}
