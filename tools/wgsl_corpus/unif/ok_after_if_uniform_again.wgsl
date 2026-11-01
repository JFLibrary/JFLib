// WGSL "Uniformity Analysis": after an if with behavior {Next} control flow is uniform again
// expect: valid
// uniformity
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  if i == 0u { }
  workgroupBarrier();
}
