// WGSL "Control Flow": discard in a function called from a fragment shader
// expect: valid
fn g(x: f32) { if x < 0.0 { discard; } }
@fragment fn fs(@location(0) x: f32) -> @location(0) vec4f { g(x); return vec4f(x); }
