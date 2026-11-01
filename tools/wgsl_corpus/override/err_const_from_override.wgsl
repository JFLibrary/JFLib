// WGSL "override Declarations": a const must not depend on an override
// expect: error
override a = 1;
const b = a + 1;
