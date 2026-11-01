// WGSL "Uniformity Analysis": the default clause depends on the selector too
// expect: error
// uniformity
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  switch i { case 0u { } default { workgroupBarrier(); } }
}
