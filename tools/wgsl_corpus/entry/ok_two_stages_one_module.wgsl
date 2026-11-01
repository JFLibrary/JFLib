// WGSL "Entry Points": vertex and fragment entry points with a matching interface
// expect: valid
struct V { @builtin(position) p: vec4f, @location(0) c: vec4f, @location(3) @interpolate(flat) k: i32 }
@vertex fn vs(@location(0) pos: vec4f) -> V { return V(pos, pos, 1); }
@fragment fn fs(v: V) -> @location(0) vec4f { return v.c * f32(v.k); }
