// WGSL "Uniformity Analysis": the result comes from an atomic
// expect: error
// uniformity
var<workgroup> a: atomic<u32>;
fn f() -> u32 { return atomicLoad(&a); }
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  if f() == 0u { workgroupBarrier(); }
}
