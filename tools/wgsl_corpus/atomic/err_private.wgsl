// WGSL "Atomic Built-in Functions": no atomics in private memory
// expect: error
var<private> a: atomic<u32>;
