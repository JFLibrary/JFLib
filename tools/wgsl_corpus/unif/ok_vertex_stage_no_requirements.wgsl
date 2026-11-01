// WGSL "Uniformity Analysis": nothing in this vertex shader needs uniformity
// expect: valid
// uniformity
@vertex fn vs(@builtin(vertex_index) v: u32) -> @builtin(position) vec4f {
  var p = vec4f();
  if v == 0u { p = vec4f(1.0); }
  return p;
}
