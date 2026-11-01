// WGSL "Constructible Types": an array of atomics is not constructible
// expect: error
fn f() { let a = array<atomic<u32>, 2>(); }
