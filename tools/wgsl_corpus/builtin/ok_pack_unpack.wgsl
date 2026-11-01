// WGSL "Built-in Functions": pack4x8snorm, pack2x16float, unpack2x16float, unpack4x8unorm
// expect: valid
fn f(v: vec4f) -> vec4f { let a = pack4x8snorm(v); let b = pack2x16float(v.xy); return vec4f(unpack2x16float(a ^ b), 0.0, 0.0) + unpack4x8unorm(a); }
