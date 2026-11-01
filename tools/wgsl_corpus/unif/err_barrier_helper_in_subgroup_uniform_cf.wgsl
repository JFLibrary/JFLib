// WGSL "Uniformity Analysis": a helper with a barrier
// expect: error
// uniformity
enable subgroups;
fn f() { workgroupBarrier(); }
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  if subgroupAny(true) { f(); }
}
