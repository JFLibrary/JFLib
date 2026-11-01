// WGSL "Uniformity Analysis": a uniform left side keeps the right side uniform
// expect: valid
// uniformity
@group(0) @binding(0) var<uniform> u: u32;
fn f() -> bool { workgroupBarrier(); return true; }
@compute @workgroup_size(64) fn main(@builtin(local_invocation_index) i: u32) {
  if u == 0u && f() { }
}
