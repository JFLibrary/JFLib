// WGSL "Uniformity Analysis": arrayLength through a pointer parameter
// expect: valid
// uniformity
@group(0) @binding(0) var<storage, read_write> rw: array<u32>;
fn g(p: ptr<storage, array<u32>, read_write>) -> u32 { return arrayLength(p); }
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  if g(&rw) > 4u { workgroupBarrier(); }
}
