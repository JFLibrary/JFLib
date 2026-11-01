// WGSL "Uniformity Analysis": a read-only storage buffer is uniform
// expect: valid
// uniformity
@group(0) @binding(0) var<storage> b: u32;
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  if b == 0u { workgroupBarrier(); }
}
