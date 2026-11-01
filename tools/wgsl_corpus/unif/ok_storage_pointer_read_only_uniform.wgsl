// WGSL "Uniformity Analysis": with a uniform element
// expect: valid
// uniformity
@group(0) @binding(0) var<storage> buf: array<u32, 64>;
fn g(p: ptr<storage, u32, read>) { if *p == 0u { workgroupBarrier(); } }
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  g(&buf[3]);
}
