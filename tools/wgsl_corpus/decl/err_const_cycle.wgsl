// WGSL "Declaration and Scope": module-scope declarations must not depend on themselves
// expect: error
const a = b + 1;
const b = a + 1;
