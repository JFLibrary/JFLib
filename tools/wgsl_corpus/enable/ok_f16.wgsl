// WGSL "Enable Extensions": enable f16: the f16 type and h literals
// expect: valid
enable f16;
fn f(x: f16) -> f16 { let y: f16 = 1.5h; return x * y + 2; }
