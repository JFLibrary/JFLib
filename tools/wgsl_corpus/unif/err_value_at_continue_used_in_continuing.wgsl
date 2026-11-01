// WGSL "Uniformity Analysis": the value at a continue reaches the continuing block
// expect: error
// uniformity
@group(0) @binding(0) var<uniform> u: u32;
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  var x = 0u;
  loop {
    x = 0u;
    if u == 0u { x = i; continue; }
    continuing { if x == 0u { workgroupBarrier(); } break if u == 3u; }
  }
}
