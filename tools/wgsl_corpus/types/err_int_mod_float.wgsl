// WGSL "Structure Types": % on floats exists (truncating), on mixed types it does not
// expect: error
fn f(a: f32, b: i32) -> f32 { return a % b; }
