// WGSL "Uniformity Analysis": a call with a pointer to the whole variable overwrites it
// expect: valid
// uniformity
fn w0(p: ptr<function, u32>) { *p = 0u; }
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  var x = i;
  w0(&x);
  if x == 0u { workgroupBarrier(); }
}
