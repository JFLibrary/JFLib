// WGSL "Uniformity Analysis": overrides are uniform
// expect: valid
// uniformity
override o: u32 = 1u;
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  if o == 0u { workgroupBarrier(); }
}
