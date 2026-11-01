// WGSL "Uniformity Analysis": a switch on a non-uniform selector
// expect: error
// uniformity
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  switch i { case 0u { workgroupBarrier(); } default { } }
}
