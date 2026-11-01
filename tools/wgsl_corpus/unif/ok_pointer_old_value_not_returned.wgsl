// WGSL "Uniformity Analysis": only the returned old value carries the non-uniformity
// expect: valid
// uniformity
fn w(p: ptr<function, u32>) -> u32 { let old = *p; *p = 0u; return old; }
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  var a = i;
  let o = w(&a);
  if a == 0u { workgroupBarrier(); }
}
