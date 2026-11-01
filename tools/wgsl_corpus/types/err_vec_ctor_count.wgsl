// WGSL "Constructible Types": a vector constructor needs exactly its components
// expect: error
fn f() { let v = vec3f(1.0, 2.0); }
