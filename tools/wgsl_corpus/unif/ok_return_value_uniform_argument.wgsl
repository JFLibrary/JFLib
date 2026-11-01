// WGSL "Uniformity Analysis": with a uniform argument the result is uniform
// expect: valid
// uniformity
fn id(x: u32) -> u32 { return x; }
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  if id(5u) == 0u { workgroupBarrier(); }
}
