// WGSL "Control Flow": discard only in fragment shaders
// expect: error
@compute @workgroup_size(1) fn main() { discard; }
