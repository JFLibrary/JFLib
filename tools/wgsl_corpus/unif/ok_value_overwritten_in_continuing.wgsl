// WGSL "Uniformity Analysis": the continuing block overwrites the value of the continue
// expect: valid
// uniformity
@group(0) @binding(0) var<uniform> u: u32;
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  var x = 0u;
  loop {
    if x == 0u { workgroupBarrier(); }
    if u == 0u { x = i; continue; }
    if u == 1u { break; }
    continuing { x = 0u; }
  }
}
