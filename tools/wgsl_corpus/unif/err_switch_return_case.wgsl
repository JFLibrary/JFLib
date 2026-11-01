// WGSL "Uniformity Analysis": a return in a case on a non-uniform selector
// expect: error
// uniformity
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  switch i { case 0u { return; } default { } }
  workgroupBarrier();
}
