// WGSL "Uniformity Analysis": an if on a non-uniform value makes its body non-uniform control flow
// expect: error
// uniformity
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  if i == 0u { workgroupBarrier(); }
}
