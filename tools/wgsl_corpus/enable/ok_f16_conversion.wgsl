// WGSL "Enable Extensions": explicit f16 <-> f32 conversions; f16 in storage and IO
// expect: valid
enable f16;
@group(0) @binding(0) var<storage, read_write> b: array<f16>;
@fragment fn fs(@location(0) x: f16) -> @location(0) vec4h { b[0] = f16(f32(x) * 2.0); return vec4h(x); }
