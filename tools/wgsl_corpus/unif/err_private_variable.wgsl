// WGSL "Uniformity Analysis": a read of a private variable may be non-uniform
// expect: error
// uniformity
var<private> pv: u32;
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  if pv == 0u { workgroupBarrier(); }
}
