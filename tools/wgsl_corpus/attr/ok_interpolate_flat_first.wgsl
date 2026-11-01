// WGSL "Attributes": @interpolate(flat, first) and (perspective, sample)
// expect: valid
@fragment fn fs(@location(0) @interpolate(flat, first) x: f32, @location(1) @interpolate(perspective, sample) y: f32) -> @location(0) vec4f { return vec4f(x + y); }
