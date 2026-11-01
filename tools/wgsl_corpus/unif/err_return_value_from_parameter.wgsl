// WGSL "Uniformity Analysis": the result depends on the parameter
// expect: error
// uniformity
fn id(x: u32) -> u32 { return x; }
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  if id(i) == 0u { workgroupBarrier(); }
}
