// WGSL "Uniformity Analysis": the result depends on the parameter through control flow
// expect: error
// uniformity
fn f(x: u32) -> u32 { if x == 0u { return 1u; } return 2u; }
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  if f(i) == 0u { workgroupBarrier(); }
}
