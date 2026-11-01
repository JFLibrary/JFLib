// WGSL "Uniformity Analysis": an atomic result is not uniform
// expect: error
// uniformity
var<workgroup> a: atomic<u32>;
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  if atomicLoad(&a) > 4u { workgroupBarrier(); }
}
