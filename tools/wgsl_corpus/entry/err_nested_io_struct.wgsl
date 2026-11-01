// WGSL "Entry Points": IO structs do not nest
// expect: error
struct Inner { @location(0) a: f32 }
struct Outer { i: Inner }
@fragment fn fs(o: Outer) -> @location(0) vec4f { return vec4f(); }
