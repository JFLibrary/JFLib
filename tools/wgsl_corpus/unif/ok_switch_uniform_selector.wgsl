// WGSL "Uniformity Analysis": a switch on a uniform selector
// expect: valid
// uniformity
@group(0) @binding(0) var<uniform> u: u32;
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  switch u { case 0u { workgroupBarrier(); } default { storageBarrier(); } }
}
