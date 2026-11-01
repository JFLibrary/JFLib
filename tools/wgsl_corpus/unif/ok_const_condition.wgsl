// WGSL "Uniformity Analysis": constants are uniform
// expect: valid
// uniformity
const c = 1u;
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  if c == 0u { workgroupBarrier(); }
}
