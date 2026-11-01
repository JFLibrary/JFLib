// WGSL "Uniformity Analysis": a body with behavior {Break} runs once: no dependency between iterations
// expect: valid
// uniformity
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  loop { workgroupBarrier(); if i == 0u { break; } else { break; } }
}
