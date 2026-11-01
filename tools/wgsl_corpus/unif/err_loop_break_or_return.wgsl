// WGSL "Uniformity Analysis": a body with behavior {Break, Return}: control flow after the loop depends on the condition
// expect: error
// uniformity
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  loop { if i == 0u { return; } else { break; } }
  workgroupBarrier();
}
