// WGSL "Uniformity Analysis": and for a function's result
// expect: error
// uniformity
@group(0) @binding(0) var<uniform> ub: array<vec4u, 4>;
fn g(p: ptr<uniform, vec4u>) -> u32 { return (*p).x; }
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  if g(&ub[i % 4u]) == 0u { workgroupBarrier(); }
}
