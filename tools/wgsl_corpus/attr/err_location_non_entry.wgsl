// WGSL "Attributes": @location on a parameter of an ordinary function
// expect: error
fn f(@location(0) x: f32) -> f32 { return x; }
