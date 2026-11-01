// WGSL "Uniformity Analysis": the same for a uniform buffer
// expect: error
// uniformity
@group(0) @binding(0) var<uniform> ub: array<vec4u, 4>;
fn g(p: ptr<uniform, vec4u>) { if (*p).x == 0u { workgroupBarrier(); } }
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  g(&ub[i % 4u]);
}
