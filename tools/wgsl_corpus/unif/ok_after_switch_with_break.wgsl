// WGSL "Uniformity Analysis": break leaves the switch: its behavior is {Next} and control flow uniform again
// expect: valid
// uniformity
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  switch i { case 0u { break; } default { } }
  workgroupBarrier();
}
