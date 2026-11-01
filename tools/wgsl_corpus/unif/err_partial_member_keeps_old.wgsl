// WGSL "Uniformity Analysis": assigning through a member keeps the old value
// expect: error
// uniformity
struct S { a: u32, b: u32 }
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  var v = S(i, i);
  v.a = 0u;
  v.b = 0u;
  if v.a == 0u { workgroupBarrier(); }
}
