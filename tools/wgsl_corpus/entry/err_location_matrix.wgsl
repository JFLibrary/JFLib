// WGSL "Entry Points": a location is a scalar or vector (no matrix)
// expect: error
@fragment fn fs(@location(0) m: mat2x2f) -> @location(0) vec4f { return vec4f(); }
