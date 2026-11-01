// WGSL "Uniformity Analysis": a write through it does not matter
// expect: valid
// uniformity
@group(0) @binding(0) var<storage, read_write> b: u32;
fn g(p: ptr<storage, u32, read_write>) { *p = 0u; }
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  g(&b);
  workgroupBarrier();
}
