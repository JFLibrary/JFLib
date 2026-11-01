// WGSL "Attributes": @interpolate(flat, centroid) is not allowed
// expect: error
@fragment fn fs(@location(0) @interpolate(flat, centroid) x: f32) -> @location(0) vec4f { return vec4f(x); }
