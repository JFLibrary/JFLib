// WGSL "Uniformity Analysis": callees first: the failure in the helper is found before the one in the entry point
// expect: error
// uniformity
@group(0) @binding(0) var<storage, read_write> rw: u32;
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  if i == 0u { storageBarrier(); }
  g();
}
fn g() { if rw == 0u { workgroupBarrier(); } }
