// WGSL "Entry Points": a location is numeric (no bool)
// expect: error
@fragment fn fs(@location(0) @interpolate(flat) b: bool) -> @location(0) vec4f { return vec4f(); }
