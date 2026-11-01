// WGSL "Uniformity Analysis": a non-uniform break later in the loop body makes the start of the next iteration non-uniform
// expect: error
// uniformity
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  loop { workgroupBarrier(); if i == 0u { break; } }
}
