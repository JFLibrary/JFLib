// WGSL "Uniformity Analysis": a quad function in non-uniform control flow
// expect: error
// uniformity
enable subgroups;
@fragment fn fs(@location(0) x: f32) -> @location(0) vec4f {
  var c = 0.0;
  if x > 0.5 { c = quadSwapX(1.0); }
  return vec4f(c);
}
