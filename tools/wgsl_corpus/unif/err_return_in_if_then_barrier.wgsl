// WGSL "Uniformity Analysis": a return in non-uniform control flow makes the rest of the function non-uniform
// expect: error
// uniformity
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  if i == 0u { return; }
  workgroupBarrier();
}
