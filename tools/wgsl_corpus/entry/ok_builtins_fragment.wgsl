// WGSL "Built-in Values": fragment built-ins: front_facing, sample_index, sample_mask, frag_depth
// expect: valid
struct O { @location(0) c: vec4f, @builtin(frag_depth) d: f32, @builtin(sample_mask) m: u32 }
@fragment fn fs(@builtin(front_facing) ff: bool, @builtin(sample_index) si: u32, @builtin(sample_mask) mi: u32) -> O {
  return O(vec4f(select(0.0, 1.0, ff)), 0.5, mi & ~si);
}
