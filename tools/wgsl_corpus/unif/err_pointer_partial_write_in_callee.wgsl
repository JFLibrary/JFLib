// WGSL "Uniformity Analysis": the callee writes a part of the pointee
// expect: error
// uniformity
fn w(p: ptr<function, vec2u>, v: u32) { (*p).x = v; }
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  var a = vec2u();
  w(&a, i);
  if a.y == 0u { workgroupBarrier(); }
}
