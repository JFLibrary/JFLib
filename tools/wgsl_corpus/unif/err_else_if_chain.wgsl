// WGSL "Uniformity Analysis": an else-if chain: the later branch depends on every earlier condition
// expect: error
// uniformity
@group(0) @binding(0) var<uniform> u: u32;
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  if u == 0u { } else if i == 0u { } else { workgroupBarrier(); }
}
