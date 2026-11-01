// WGSL "Uniformity Analysis": a return in a loop: control flow after the loop stays non-uniform
// expect: error
// uniformity
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  loop { if i == 0u { return; } if i == 1u { break; } }
  workgroupBarrier();
}
